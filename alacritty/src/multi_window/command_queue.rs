use std::sync::Arc;
use mio_extras::channel::Sender;
use glutin::event_loop::EventLoopWindowTarget;
use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;

use crate::config::Config;
use crate::display;
use crate::multi_window::term_tab::MultiWindowEvent;
use crate::multi_window::window_context_tracker::WindowContext;

use crate::event::EventProxy;
use crate::multi_window::window_context_tracker::WindowContextTracker;

#[derive(Clone, PartialEq)]
pub enum MultiWindowCommand {
    NewWindow,
    CreateTab,
    ActivateTab(usize), // tab_id
    CloseCurrentTab,
    CloseTab(usize), // tab_id
}

#[derive(Default)]
pub struct MultiWindowCommandQueue {
    queue: Vec<MultiWindowCommand>,
    has_create: bool,
}

impl MultiWindowCommandQueue {
    pub fn push(&mut self, command: MultiWindowCommand) {
        if command == MultiWindowCommand::NewWindow {
            self.has_create = true;
        }

        self.queue.push(command);
    }

    pub fn run_user_input_commands<'a>(
        &mut self,
        context_tracker: &mut WindowContextTracker,
        window_ctx: &WindowContext,
        config: &'a mut Config,
        window_event_loop: &EventLoopWindowTarget<Event>,
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<(), display::Error> {
        let config_arc = Arc::new(FairMutex::new(config));
        let need_redraw = !self.queue.is_empty();

        for command in self.queue.drain(..) {
            match command {
                MultiWindowCommand::NewWindow => {
                    let mut config = config_arc.lock();
                    context_tracker.create_window_context(
                        &mut config,
                        window_event_loop,
                        event_proxy,
                        dispatcher.clone()
                    )?;
                }
                MultiWindowCommand::CreateTab => {
                    let size_info = window_ctx.processor.lock().get_size_info();
                    let config = config_arc.lock();
                    let mut tab_collection = window_ctx.term_tab_collection.lock();

                    let tab_id = tab_collection.add_tab(
                        &config,
                        size_info,
                        Some(window_ctx.window_id),
                        &dispatcher,
                    );

                    tab_collection.activate_tab(tab_id);
                }
                MultiWindowCommand::ActivateTab(tab_id) => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.activate_tab(tab_id);
                }
                MultiWindowCommand::CloseCurrentTab => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_current_tab();
                }
                MultiWindowCommand::CloseTab(tab_id) => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_tab(tab_id);
                }
            };
        }

        if window_ctx.term_tab_collection.lock().is_empty() {
            context_tracker.close_window(window_ctx.window_id);
        } else if need_redraw {
            let mut terminal = {
                let tab_collection = window_ctx.term_tab_collection.lock();
                let active_tab = tab_collection.active_tab().unwrap();
                active_tab.terminal.clone()
            };

            let mut processor = window_ctx.processor.lock();
            let config = config_arc.lock();
            processor.update_size(&mut terminal.lock(), &config);
            processor.request_redraw();
        }

        Ok(())
    }
}
