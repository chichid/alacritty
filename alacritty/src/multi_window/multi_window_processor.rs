use glutin::window::WindowId;
use alacritty_terminal::event_loop::Notifier;
use mio_extras::channel::{self, Receiver};

use glutin::event_loop::ControlFlow;
use glutin::event::Event as GlutinEvent;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;
use alacritty_terminal::event::Event;

use crate::multi_window::term_tab::MultiWindowEvent;
use crate::config::Config;
use crate::event::EventProxy;
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

        let mut window_context_tracker = WindowContextTracker::new();
        window_context_tracker.initialize(
            &mut config, 
            &window_event_loop, 
            &event_proxy, 
            multi_window_tx.clone()
        )?;

        let mut schedule_window_activation: Option<WindowId> = None;

        // Run the process event loop
        window_event_loop.run_return(move |event, event_loop, mut control_flow| {
            // Activation, Deactivation and closing of windows
            if self.handle_multi_window_events(
                event.clone(),
                &mut control_flow,
                &mut event_queue,
                &mut window_context_tracker,
                &mut schedule_window_activation,
            ) { return; }

            // PTY Detach for all windows and dirty state for inactive terminals
            if self.handle_pty_events(
                &config,
                &mut window_context_tracker,
                &multi_window_rx,
            ) == None { return; }

            // If we closed all the windows
            if window_context_tracker.is_empty() {
                *control_flow = ControlFlow::Exit;
                return;
            }

            // If nothing is active, process the inactive windows
            // otherwise process the active window first, then draw the inactive windows
            if !window_context_tracker.has_active_window() {
                self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
                return;
            }
        
            let active_ctx = window_context_tracker.get_active_window_context();
            let active_tab = active_ctx.active_tab();
            if active_tab.is_none() {
                self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
               return;
            };

            let mut multi_window_command_queue = {
                let mut command_queue = MultiWindowCommandQueue::default();
                let mut processor = active_ctx.processor.lock();

                // Handle Tab-bar events
                let size_info = processor.get_size_info();
                let (need_redraw, skip_processor_run, cursor_icon) = {
                    active_ctx.tab_bar_processor.lock().handle_event(
                        &active_ctx.term_tab_collection.lock(),
                        &config, 
                        &size_info, 
                        event.clone(),
                        &mut command_queue,
                    )
                };

                if let Some(cursor_icon) = cursor_icon {
                    processor.window_mut().set_mouse_cursor(cursor_icon);
                }

                if need_redraw {
                    processor.request_redraw();
                }

                if !skip_processor_run {
                    let active_tab = active_tab.unwrap();
                    let mut notifier = Notifier(active_tab.loop_tx.clone());
            
                    processor.make_current();
                    
                    processor.run_iteration(
                        &mut notifier,
                        &mut event_queue,
                        event.clone(),
                        &mut control_flow,
                        active_tab.terminal,
                        &mut config,
                        &mut command_queue,
                    );
                    
                }

                command_queue
            };

            // let active_ctx = window_context_tracker.get_active_window_context();
            match multi_window_command_queue.run_user_input_commands(
                &mut window_context_tracker,
                &active_ctx,
                &mut config,
                &event_loop,
                &event_proxy,
                multi_window_tx.clone(),
            ) {
                Ok(_) => {}
                Err(_err) => {
                    // TODO log error
                }
            };

            // Handle windows that are visible but not active
            self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
        });

        Ok(())
    }

    fn handle_pty_events(
        &self,
        config: &Config,
        context_tracker: &mut WindowContextTracker, 
        receiver: &Receiver<MultiWindowEvent>
    ) -> Option<bool> {
        match receiver.try_recv() {
            Ok(result) => {
                let window_id = result.window_id?;
                let tab_id = result.tab_id;
                let ctx = context_tracker.get_context(window_id)?;

                match result.wrapped_event {
                    Event::Exit => {
                        let should_exit = {
                            let mut tab_collection = ctx.term_tab_collection.lock();
                            tab_collection.close_tab(tab_id);
                            tab_collection.is_empty()
                        };
                        
                        if should_exit {
                            context_tracker.close_window(window_id);
                        } else {
                            // Redraw the active tab
                            let active_tab = ctx.active_tab()?;
                            let mut terminal = active_tab.terminal.lock();
                            let mut processor = ctx.processor.lock();
                            processor.update_size(&mut terminal, config);
                            processor.request_redraw();
                        }

                        return None;
                    }

                    Event::Title(title) => {
                        ctx.term_tab_collection.lock().tab_mut(tab_id).set_title(title);
                        ctx.processor.lock().request_redraw();
                    }

                    _ => {}
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
        control_flow: &mut ControlFlow,
        event_queue: &mut Vec<GlutinEvent<Event>>,
        context_tracker: &mut WindowContextTracker,
        schedule_window_activation: &mut Option<WindowId>,
    ) -> bool {
        
        match event {
            // Process events
            GlutinEvent::EventsCleared => {
                *control_flow = ControlFlow::Wait;

                if event_queue.is_empty() {
                    return true;
                }
            },

            // Handle Window Activate, Deactivate, Close Events
            GlutinEvent::WindowEvent { event, window_id, .. } => {
                use glutin::event::WindowEvent::*;

                match event {
                    Focused(is_focused) => {
                        if is_focused {
                            // Do not activate the window right away, this causes weird selection behaviour
                            // wait until the mouse_input is received on the next iteration or 
                            *schedule_window_activation = Some(window_id);
                            *control_flow = ControlFlow::Poll;
                            return true;
                        } else {
                            context_tracker.deactivate_window(window_id);
                        }
                    },

                    CloseRequested => {
                        context_tracker.close_window(window_id);
                    },

                    _ => {
                    }
                }
            },

            _ => {} 
        }

        if *schedule_window_activation != None {
            context_tracker.activate_window(schedule_window_activation.unwrap());

            *schedule_window_activation = None;
            *control_flow = ControlFlow::Wait;
            
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

        for inactive_ctx in context_tracker.get_all_window_contexts() {
            // TODO check if the window related to the context is maximized
           if !has_active_display  || inactive_ctx.window_id != active_window_id.unwrap() {
               let tab = inactive_ctx.active_tab().unwrap();
               if tab.terminal.lock().dirty {   
                   tab.terminal.lock().dirty = false;
                   inactive_ctx.processor.lock().redraw(tab.terminal.clone(), config);
               }
           }
        }
    }
}