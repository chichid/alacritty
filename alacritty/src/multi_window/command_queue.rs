use glutin::event_loop::EventLoopWindowTarget;
use glutin::event::{Event as GlutinEvent};

use alacritty_terminal::term::SizeInfo;
use alacritty_terminal::event::Event;

use crate::display;
use crate::config::Config;

use crate::multi_window::window_context_tracker::WindowContextTracker;
use crate::event::EventProxy;

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
                    context_tracker.activate_window(*window_id);
                } else {
                    context_tracker.deactivate_window(*window_id);
                }
            },
            CloseRequested => {
                is_close_requested = true;
                context_tracker.close_window(*window_id);
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
            context_tracker.close_window(win_id.unwrap());
        }
    }
    
    if context_tracker.is_empty() {
      return DisplayCommandResult::Exit
    }

    DisplayCommandResult::Continue
  }

  pub fn run_user_input_commands(&mut self,
    context_tracker: &mut WindowContextTracker, 
    size_info: SizeInfo,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<bool, display::Error> {
    // Drain the displaycommand queue
    let mut is_dirty = false;
    let current_display_ctx = context_tracker.get_active_display_context();
    let current_tab_collection = &mut current_display_ctx.term_tab_collection.lock();

    for command in self.queue.iter() {
      let mut did_run_command = true;

      match command {
        DisplayCommand::CreateDisplay => { context_tracker.create_display(config, window_event_loop, event_proxy)?; },
        DisplayCommand::CreateTab => { current_tab_collection.push_tab(); },
        DisplayCommand::ActivateTab(tab_id) => { current_tab_collection.activate_tab(*tab_id); },
        DisplayCommand::CloseCurrentTab => { current_tab_collection.close_current_tab(); },
        DisplayCommand::CloseTab(tab_id) => { current_tab_collection.close_tab(*tab_id); },
        _ => { did_run_command = false }
      }

      if did_run_command {
        is_dirty = true;
      }
    }

    // Commit any changes to the tab collection
    let is_tab_collection_dirty = current_tab_collection.commit_changes(
      config, 
      size_info,
    );

    Ok(is_dirty || is_tab_collection_dirty)
  }
}