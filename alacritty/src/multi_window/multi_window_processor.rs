use crate::config::Config;
use crate::event::EventProxy;
use glutin::event_loop::ControlFlow;

use alacritty_terminal::event::Event;
use glutin::event::Event as GlutinEvent;

use alacritty_terminal::message_bar::MessageBuffer;

use crate::event::Processor;
use crate::multi_window::command_queue::{
    MultiWindowCommand, MultiWindowCommandQueue, MultiWindowCommandResult,
};
use crate::multi_window::window_context_tracker::WindowContextTracker;

use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;

#[derive(Default)]
pub struct MultiWindowProcessor {}

impl MultiWindowProcessor {
    pub fn run(
        &self,
        mut config: Config,
        mut window_event_loop: GlutinEventLoop<Event>,
        mut context_tracker: WindowContextTracker,
        event_proxy: EventProxy,
    ) {
        // Setup shared storage for message UI
        let message_buffer = MessageBuffer::new();

        // Shared User Input Event processor
        //
        // Need the Rc<RefCell<_>> here since a ref is shared in the resize callback
        // TODO investigate making this window specific instead of shared
        let mut processor = Processor::new(message_buffer, config.font.size);

        // Event queue
        //
        // TODO investigate making this window specific instead of shared
        let mut event_queue = Vec::new();

        window_event_loop.run_return(|event, _event_loop, mut control_flow| {
            let mut multi_window_queue = MultiWindowCommandQueue::default();

            // Activation & Deactivation of windows
            let should_return = self.handle_events(
                &event,
                &mut control_flow,
                &mut context_tracker,
                &mut multi_window_queue,
            );

            if should_return {
                return;
            }

            if !context_tracker.has_active_window() {
                return;
            }

            // Process events for the active display, user input etc.
            let mut active_ctx = context_tracker.get_active_window_context();

            processor.run(
                &mut event_queue,
                &mut multi_window_queue,
                &mut active_ctx,
                event,
                &mut control_flow,
                &mut config,
            );

            // Process input events and drawing of the main screen
            match multi_window_queue.run_user_input_commands(
                &mut context_tracker,
                &mut active_ctx,
                &config,
                _event_loop,
                &event_proxy,
            ) {
                Ok(_) => {}
                Err(_err) => {}
            };

            // Draw the inactive visible windows
            for inactive_ctx in context_tracker.get_all_window_contexts() {
                // TODO check if the display is not minimized
                if inactive_ctx.window_id != active_ctx.window_id {
                    let display = inactive_ctx.display.lock();
                    let tab = inactive_ctx.get_active_tab();
                    let terminal = tab.terminal.lock();

                    if terminal.dirty {
                        println!("Dirty inactive Terminal found");
                    }
                }
            }
        });
    }

    fn handle_events(
        &self,
        event: &GlutinEvent<Event>,
        control_flow: &mut ControlFlow,
        context_tracker: &mut WindowContextTracker,
        window_command_queue: &mut MultiWindowCommandQueue,
    ) -> bool {
        use glutin::event::WindowEvent::*;

        let mut is_close_requested = false;
        let mut win_id = None;

        // Handle Window Activate, Deactivate, Close Events
        if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
            win_id = Some(*window_id);

            match event {
                Focused(is_focused) => {
                    if *is_focused {
                        context_tracker.activate_window(*window_id);
                    } else {
                        context_tracker.deactivate_window(*window_id);
                    }
                }
                CloseRequested => {
                    is_close_requested = true;
                    context_tracker.close_window(*window_id);
                }
                _ => {}
            }
        }

        // handle pty detach (ex. when user types exit)
        if let GlutinEvent::UserEvent(Event::Exit) = &event {
            if !is_close_requested {
                window_command_queue.push(MultiWindowCommand::CloseCurrentTab);
            }
        }

        // Handle Closing all the tabs within a window (close the window)
        if win_id != None && context_tracker.has_active_window() {
            let display_ctx = context_tracker.get_active_window_context();
            let term_tab_collection_arc = display_ctx.term_tab_collection.clone();
            let term_tab_collection = term_tab_collection_arc.lock();

            if term_tab_collection.is_empty() {
                context_tracker.close_window(win_id.unwrap());
                *control_flow = ControlFlow::Exit;
                return true;
            }
        }

        if context_tracker.is_empty() {
            return true;
        }

        false
    }
}
