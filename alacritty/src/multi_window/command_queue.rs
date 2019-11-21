use std::slice::Iter;
use glutin::event::{Event as GlutinEvent};

use alacritty_terminal::event::Event;

use crate::multi_window::window_context_tracker::WindowContextTracker;

#[derive (Clone, PartialEq)]
pub enum DisplayCommand {
  CreateDisplay,
  CreateTab,
  ActivateTab(usize), // tab_id
  CloseCurrentTab,
  CloseTab(usize),// tab_id
}

#[derive (Clone)]
pub enum DisplayCommandResult {
  Exit,
  Continue,
  RestartLoop,
  Redraw,
}

#[derive (Default)]
pub struct DisplayCommandQueue {
  queue: Vec<DisplayCommand>,
  has_create: bool,
}

impl DisplayCommandQueue {
  pub fn push(&mut self, command: DisplayCommand) {
    if command == DisplayCommand::CreateDisplay {
      self.has_create = true;
    }

    self.queue.push(command);
  }

  pub fn iterator(&self) -> Iter<DisplayCommand> {
    self.queue.iter()
  }

  pub fn has_create_display_command(&self) -> bool {
    self.has_create
  }

  pub fn handle_multi_window_events(
    &mut self, 
    context_tracker: &mut WindowContextTracker, 
    event: &GlutinEvent<Event>,
  ) -> DisplayCommandResult {
    use glutin::event::WindowEvent::*;

    let mut is_close_requested = false;
    let mut win_id = None;

    // Handle Window Activate, Deactivate, Close Events
    if let GlutinEvent::WindowEvent { event, window_id, .. } = &event {
        win_id = Some(*window_id);

        match event {
            Focused(is_focused) => {
                if *is_focused {
                    context_tracker.command_activate_window(*window_id);
                } else {
                    context_tracker.command_deactivate_window(*window_id);
                }
            },
            CloseRequested => {
                is_close_requested = true;
                context_tracker.command_close_window(*window_id);
            }
            _ => {}
        }
    }

    // handle pty detach (ex. when we type exit)
    if let GlutinEvent::UserEvent(Event::Exit) = &event {
        if !is_close_requested {
            self.push(DisplayCommand::CloseCurrentTab);
        }
    }
    
    // Handle Closing all the tabs within a window (close the window)
    if win_id != None && context_tracker.has_active_display() {
        let display_ctx = context_tracker.get_active_display_context();
        let term_tab_collection_arc = display_ctx.term_tab_collection.clone();
        let term_tab_collection = term_tab_collection_arc.lock();

        if term_tab_collection.is_empty() {
            context_tracker.command_close_window(win_id.unwrap());
        }
    }
    
    if context_tracker.is_empty() {
      return DisplayCommandResult::Exit
    }

    DisplayCommandResult::Continue
  }
}