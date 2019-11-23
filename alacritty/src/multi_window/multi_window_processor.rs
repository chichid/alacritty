use glutin::event_loop::EventLoopWindowTarget;
use mio_extras::channel::Receiver;
use mio_extras::channel::Sender;
use crate::multi_window::term_tab::MultiWindowEvent;
use crate::config::Config;
use crate::event::EventProxy;
use glutin::event_loop::ControlFlow;

use alacritty_terminal::event::Event;
use glutin::event::Event as GlutinEvent;

use alacritty_terminal::message_bar::MessageBuffer;

use crate::event::Processor;
use crate::multi_window::command_queue::{ MultiWindowCommand, MultiWindowCommandQueue };
use crate::multi_window::window_context_tracker::WindowContextTracker;
use crate::display::Error as DisplayError;

use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;
use mio_extras::channel::{self};

#[derive(Default)]
pub struct MultiWindowProcessor {}

impl MultiWindowProcessor {
    pub fn run(
        &self,
        mut config: Config,
        mut window_event_loop: GlutinEventLoop<Event>,
        event_proxy: EventProxy,
    ) -> Result<(), DisplayError> {
        let mut event_queue = Vec::new();
        let (multi_window_tx, multi_window_rx) = channel::channel(); 
        let message_buffer = MessageBuffer::new();
        let mut processor = Processor::new(message_buffer, config.font.size);

        let mut window_context_tracker = WindowContextTracker::new();
        window_context_tracker.initialize(
            &config, 
            &window_event_loop, 
            &event_proxy, 
            multi_window_tx.clone()
        )?;

        // Run the process event loop
        window_event_loop.run_return(move |event, event_loop, mut control_flow| {
            // If we closed all the windows
            if window_context_tracker.is_empty() {
                *control_flow = ControlFlow::Exit;
                return;
            }

            // Activation, Deactivation and closing of windows
            if self.handle_multi_window_events(
                event.clone(),
                control_flow,
                &mut window_context_tracker,
            ) { return; }

            // PTY Detach and dirty state for inactive terminals
            if self.handle_pty_events(
                &mut window_context_tracker,
                &multi_window_rx,
            ) { return; }

            // Handle input and drawing of the current display
            let mut window_processor = WindowProcessor {
                config: &mut config,
                processor: &mut processor,
                event_loop: &event_loop,
                event_proxy: &event_proxy,
                event_queue: &mut event_queue,
                event: event.clone(),
                control_flow: &mut control_flow,
                context_tracker: &mut window_context_tracker,
                multi_window_tx: &multi_window_tx,
            };

            window_processor.run();

            // Handle windows that are visible but not active
            self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
        });

        Ok(())
    }

    fn handle_pty_events(
        &self, 
        context_tracker: &mut WindowContextTracker, 
        receiver: &Receiver<MultiWindowEvent>
    ) -> bool {
        match receiver.try_recv() {
            Ok(result) => {
                // TODO 
                // handle pty detach (ex. when user types exit)
                // if let GlutinEvent::UserEvent(Event::Exit) = &event {
                //     if !is_close_requested {
                //         window_command_queue.push(MultiWindowCommand::CloseCurrentTab);
                //     }
                // }
                let window_id = result.window_id;
                if window_id != None {
                    let window_id = window_id.unwrap();
                    let ctx = context_tracker.get_context(window_id);
                    let active_tab = ctx.get_active_tab();

                    if active_tab.tab_id == result.tab_id {
                        let mut terminal = active_tab.terminal.lock();
                        terminal.dirty = true;
                    }
                }

                true
            },
            Err(err) => {
                // TODO log errors
                false
            }
        }
    }

    fn handle_multi_window_events(
        &self,
        event: GlutinEvent<Event>,
        control_flow: &mut ControlFlow,
        context_tracker: &mut WindowContextTracker,
    ) -> bool {
        use glutin::event::WindowEvent::*;

        let mut win_id = None;

        // Handle Window Activate, Deactivate, Close Events
        if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
            win_id = Some(window_id);

            match event {
                Focused(is_focused) => {
                    if is_focused {
                        context_tracker.activate_window(window_id);
                    } else {
                        context_tracker.deactivate_window(window_id);
                    }
                }
                CloseRequested => {
                    context_tracker.close_window(window_id);
                }
                _ => {}
            }
        }

        // If we closed all the windows
        if context_tracker.is_empty() {
            *control_flow = ControlFlow::Exit;
            return true;
        }
        
        false
    }

    fn draw_inactive_visible_windows(&self, config: &Config, context_tracker: &mut WindowContextTracker) {
        let has_active_display = context_tracker.has_active_window();
        let active_window_id = if has_active_display {
            Some( context_tracker.get_active_window_context().window_id)
        } else {
            None
        };

        let mut did_render = false;

        for inactive_ctx in context_tracker.get_all_window_contexts() {

           if !has_active_display  || inactive_ctx.window_id != active_window_id.unwrap() {
               let tab = inactive_ctx.get_active_tab();
               let mut terminal = tab.terminal.lock();

               if terminal.dirty {   
                   did_render = true;                    
                   terminal.dirty = false;

                   let mut display = inactive_ctx.display.lock();
           
                   let mouse = Default::default();
                   let modifiers = Default::default();
                   let message_buffer = MessageBuffer::new();

                   display.window.make_current();
                   
                   // Redraw screen
                   display.draw(
                       terminal,
                       &message_buffer,
                       &config,
                       &mouse,
                       modifiers,
                   ); 
               }
           }
        }

        if did_render && has_active_display {
            let active_ctx = context_tracker.get_active_window_context();
            let display = active_ctx.display.lock();
            display.window.make_current();    
        }
    }
}


struct WindowProcessor<'a> { 
    config: &'a mut Config,
    processor: &'a mut Processor,
    event_loop: &'a EventLoopWindowTarget<Event>,
    event_proxy: &'a EventProxy,
    event_queue: &'a mut Vec<GlutinEvent<Event>>,
    event: GlutinEvent<Event>,
    control_flow: &'a mut ControlFlow,
    context_tracker: &'a mut WindowContextTracker,
    multi_window_tx: &'a Sender<MultiWindowEvent>,
}

impl<'a> WindowProcessor<'a> {
    fn run(&mut self) {
         // No Active Currently so skip it!
         if !self.context_tracker.has_active_window() {
            return;
        }

        // Command queue for the multi-window commands such as create_new_window, etc.
        let mut multi_window_queue = MultiWindowCommandQueue::default();

        // Process events for the active display, user input etc.
        let mut active_context = self.context_tracker.get_active_window_context();

        self.processor.run(
            self.event_queue,
            &mut multi_window_queue,
            &mut active_context,
            self.event.clone(),
            self.control_flow,
            self.config,
        );

        match multi_window_queue.run_user_input_commands(
            self.context_tracker,
            &mut active_context,
            self.config,
            self.event_loop,
            self.event_proxy,
            self.multi_window_tx.clone(),
        ) {
            Ok(_) => {}
            Err(_err) => {
                // TODO log error
            }
        };
    }
}