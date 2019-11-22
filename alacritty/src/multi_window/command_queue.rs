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
    ) -> Result<bool, display::Error> {
        // Drain the displaycommand queue
        let mut is_dirty = false;
        let display = window_ctx.display.lock();
        let size_info = display.size_info;
        let window = &display.window;
        let current_tab_collection = &mut window_ctx.term_tab_collection.lock();

        for command in self.queue.iter() {
            let mut did_run_command = true;

            match command {
                MultiWindowCommand::CreateDisplay => {
                    context_tracker.create_display(config, window_event_loop, event_proxy)?;
                }
                MultiWindowCommand::CreateTab => {
                    current_tab_collection.push_tab();
                }
                MultiWindowCommand::ActivateTab(tab_id) => {
                    current_tab_collection.activate_tab(*tab_id);
                }
                MultiWindowCommand::CloseCurrentTab => {
                    current_tab_collection.close_current_tab();
                }
                MultiWindowCommand::CloseTab(tab_id) => {
                    current_tab_collection.close_tab(*tab_id);
                }
                _ => did_run_command = false,
            }

            if did_run_command {
                is_dirty = true;
            }

            window.request_redraw();
        }

        // Commit any changes to the tab collection
        let is_tab_collection_dirty = current_tab_collection.commit_changes(config, size_info);

        Ok(is_dirty || is_tab_collection_dirty)
    }
}
