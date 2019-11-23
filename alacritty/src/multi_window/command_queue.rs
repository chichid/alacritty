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
        let mut is_dirty = false;
        let display = window_ctx.display.lock();
        let size_info = display.size_info;
        let window = &display.window;
        let window_id = window_ctx.window_id;
        let current_tab_collection = &mut window_ctx.term_tab_collection.lock();

        for command in self.queue.iter() {
            if match command {
                MultiWindowCommand::CreateDisplay => {
                    context_tracker.create_display(config, window_event_loop, event_proxy, dispatcher.clone())?;
                    true
                }
                MultiWindowCommand::CreateTab => {
                    let tab_id = current_tab_collection.add_tab(
                        config,
                        size_info,
                        Some(window_id),
                        &dispatcher,
                    );
                    current_tab_collection.activate_tab(tab_id);
                    true
                }
                MultiWindowCommand::ActivateTab(tab_id) => {
                    current_tab_collection.activate_tab(*tab_id);
                    true
                }
                MultiWindowCommand::CloseCurrentTab => {
                    current_tab_collection.close_current_tab();
                    true
                }
                MultiWindowCommand::CloseTab(tab_id) => {
                    current_tab_collection.close_tab(*tab_id);
                    true
                }
                _ => false
            } {
                window.request_redraw();
                is_dirty = true;
            }            
        }

        // Commit any changes to the tab collection
        let is_tab_collection_dirty = current_tab_collection.commit_changes(
            Some(window_ctx.window_id),
            config, 
            size_info,
            dispatcher,
        );

        // Close the window if we closed all the tabs within a tab collection
        if current_tab_collection.is_empty() {
            print!("It's empty!!");
            return Ok(true);
        }

        Ok(is_dirty || is_tab_collection_dirty)
    }
}
