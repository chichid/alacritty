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
    CreateDisplay,
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
        if command == MultiWindowCommand::CreateDisplay {
            self.has_create = true;
        }

        self.queue.push(command);
    }

    pub fn has_create_display_command(&self) -> bool {
        self.has_create
    }

    pub fn run_user_input_commands(
        &mut self,
        context_tracker: &mut WindowContextTracker,
        window_ctx: &mut WindowContext,
        config: &Config,
        window_event_loop: &EventLoopWindowTarget<Event>,
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<bool, display::Error> {
        let mut need_redraw = false;

        for command in self.queue.iter() {
            let is_dirty = match command {
                MultiWindowCommand::CreateDisplay => {
                    context_tracker.create_display(config, window_event_loop, event_proxy, dispatcher.clone())?;

                    true
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

                    true
                }
                MultiWindowCommand::ActivateTab(tab_id) => {
                    let display = window_ctx.display.lock();
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.activate_tab(*tab_id);

                    true
                }
                MultiWindowCommand::CloseCurrentTab => {
                    let display = window_ctx.display.lock();
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_current_tab();

                    true
                }
                MultiWindowCommand::CloseTab(tab_id) => {
                    let display = window_ctx.display.lock();
                    let mut tab_collection = window_ctx.term_tab_collection.lock();
                    tab_collection.close_tab(*tab_id);

                    true
                }
                _ => false
            };
            
            if is_dirty {
                need_redraw = true;
            }
        }

        if window_ctx.term_tab_collection.lock().is_empty() {
            context_tracker.close_window(window_ctx.window_id);
            return Ok(false);
        }

        if need_redraw {
            let display = window_ctx.display.lock();
            display.window.request_redraw();
        }

        Ok(need_redraw)
    }
}
