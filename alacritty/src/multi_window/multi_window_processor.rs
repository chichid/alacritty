use crate::multi_window::window_context_tracker::WindowContext;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::event_loop::Notifier;
use std::sync::Arc;
use mio_extras::channel::{self, Receiver, Sender};

use glutin::event_loop::ControlFlow;
use glutin::event_loop::EventLoopWindowTarget;
use glutin::event::Event as GlutinEvent;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;
use alacritty_terminal::event::Event;
use alacritty_terminal::message_bar::MessageBuffer;

use crate::multi_window::term_tab::MultiWindowEvent;
use crate::config::Config;
use crate::event::EventProxy;
use crate::event::Processor;
use crate::multi_window::command_queue::{ MultiWindowCommandQueue };
use crate::multi_window::window_context_tracker::WindowContextTracker;
use crate::display::Error as DisplayError;

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
        let message_buffer = Arc::new(FairMutex::new(MessageBuffer::new()));

        let mut window_context_tracker = WindowContextTracker::new();
        window_context_tracker.initialize(
            &config, 
            &window_event_loop, 
            &event_proxy, 
            multi_window_tx.clone()
        )?;

        // Run the process event loop
        window_event_loop.run_return(move |event, event_loop, mut control_flow| {
            // Command queue for the multi-window commands such as create_new_window, etc.
            let mut multi_window_queue = MultiWindowCommandQueue::default();

            // Activation, Deactivation and closing of windows
            if self.handle_multi_window_events(
                event.clone(),
                &mut window_context_tracker,
            ) { return; }

            // PTY Detach for all windows and dirty state for inactive terminals
            if self.handle_pty_events(
                &mut window_context_tracker,
                &multi_window_rx,
            ) == None { return; }

            // If we closed all the windows
            if window_context_tracker.is_empty() {
                *control_flow = ControlFlow::Exit;
                return;
            }

            // If nothing is active, only process the inactive windows
            // otherwise process the active window first, then draw the inactive windows
            if !window_context_tracker.has_active_window() {
                self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
                return;
            }
        
            // Handle input and drawing of the current display
            let mut window_processor = WindowProcessor {
                active_context: &mut window_context_tracker.get_active_window_context(),
                config: &mut config,
                event_loop: &event_loop,
                event_proxy: &event_proxy,
                event_queue: &mut event_queue,
                message_buffer_arc: message_buffer.clone(),
                event: event.clone(),
                control_flow: &mut control_flow,
                context_tracker: &mut window_context_tracker,
                multi_window_tx: &multi_window_tx,
                multi_window_queue: &mut multi_window_queue,
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
    ) -> Option<bool> {
        match receiver.try_recv() {
            Ok(result) => {
                let ctx = context_tracker.get_context(result.window_id?)?;
                if result.wrapped_event == Event::Exit {
                    let tab_id = result.tab_id;
                    let mut tab_collection = ctx.term_tab_collection.lock();
                    tab_collection.close_tab(tab_id);
                    return None;
                }
                
                let active_tab = ctx.get_active_tab()?;
                if active_tab.tab_id == result.tab_id {
                    let mut terminal = active_tab.terminal.lock();
                    terminal.dirty = true;
                }

                Some(true)
            },
            Err(err) => {
                // TODO log errors
                // change the result of this function to be Result once that's done
                Some(true)
            }
        }
    }

    fn handle_multi_window_events(
        &self,
        event: GlutinEvent<Event>,
        context_tracker: &mut WindowContextTracker,
    ) -> bool {
        use glutin::event::WindowEvent::*;

        // Handle Window Activate, Deactivate, Close Events
        if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
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
            // TODO check if the window related to the context is maximized
           if !has_active_display  || inactive_ctx.window_id != active_window_id.unwrap() {
               let tab = inactive_ctx.get_active_tab().unwrap();
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

        if did_render && context_tracker.has_active_window() {
            let active_ctx = context_tracker.get_active_window_context();
            let display = active_ctx.display.lock();
            display.window.make_current();
        }
    }
}

struct WindowProcessor<'a> {
    active_context: &'a mut WindowContext,
    config: &'a mut Config,
    event_loop: &'a EventLoopWindowTarget<Event>,
    event_proxy: &'a EventProxy,
    event_queue: &'a mut Vec<GlutinEvent<Event>>,
    message_buffer_arc: Arc<FairMutex<MessageBuffer>>,
    event: GlutinEvent<Event>,
    control_flow: &'a mut ControlFlow,
    context_tracker: &'a mut WindowContextTracker,
    multi_window_tx: &'a Sender<MultiWindowEvent>,
    multi_window_queue: &'a mut MultiWindowCommandQueue,
}

impl<'a> WindowProcessor<'a> {
    fn run(&mut self) {
        self.run_processor();
        self.run_multi_window_input_commands();
    }

    fn run_processor(&mut self) {
        let mut display = self.active_context.display.lock();
        let active_tab = self.active_context.get_active_tab();

        if active_tab.is_none() { return; }

        let active_tab = active_tab.unwrap();
        let notifier = Notifier(active_tab.loop_tx.clone());
        let mut pty_resize_handle = active_tab.resize_handle.lock();
        let mut message_buffer = self.message_buffer_arc.lock();
        let term_arc = active_tab.terminal;
        
        display.window.make_current();

        let mut processor = Processor::new(
            self.multi_window_queue,
            notifier, 
            &mut pty_resize_handle, 
            &mut message_buffer, 
            self.config,
            &mut display,            
        );

        processor.run_iteration(
            self.event_queue,
            self.event.clone(),
            self.control_flow,
            term_arc,
        );
    }

    fn run_multi_window_input_commands(&mut self) {
        match self.multi_window_queue.run_user_input_commands(
            self.context_tracker,
            &self.active_context,
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