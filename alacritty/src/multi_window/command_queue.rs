use mio_extras::channel::Sender;
use crate::multi_window::term_tab::MultiWindowEvent;
use crate::multi_window::window_context_tracker::WindowContext;
use alacritty_terminal::event::Event;
use glutin::event_loop::EventLoopWindowTarget;

use crate::config::Config;
use crate::display;

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

#[derive(Clone)]
pub enum MultiWindowCommandResult {
    Exit,
    Continue,
    RestartLoop,
    Redraw,
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

    pub fn run_user_input_commands(
        &mut self,
        context_tracker: &mut WindowContextTracker,
        window_ctx: &WindowContext,
        config: &Config,
        window_event_loop: &EventLoopWindowTarget<Event>,
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<(), display::Error> {
        for command in self.queue.iter() {
            match command {
                MultiWindowCommand::NewWindow => {
                    context_tracker.create_window_context(config, window_event_loop, event_proxy, dispatcher.clone())?;
                }
                MultiWindowCommand::CreateTab => {
                    let display = window_ctx.display.lock();
                    let mut tab_collection = window_ctx.term_tab_collection.lock();

                    let tab_id = tab_collection.add_tab(
                        config,
                        display.size_info,
                        Some(window_ctx.window_id),
                        &dispatcher,
                    );

                    tab_collection.activate_tab(tab_id);
                }
                MultiWindowCommand::ActivateTab(tab_id) => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.activate_tab(*tab_id);
                }
                MultiWindowCommand::CloseCurrentTab => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_current_tab();
                }
                MultiWindowCommand::CloseTab(tab_id) => {
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_tab(*tab_id);
                }
            };
        }

        if window_ctx.term_tab_collection.lock().is_empty() {
            context_tracker.close_window(window_ctx.window_id);
        } else if !self.queue.is_empty() {
            let display = window_ctx.display.lock();
            display.window.request_redraw();
        }

        Ok(())
    }
}
