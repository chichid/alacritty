use std::slice::Iter;
use std::slice::IterMut;
use glutin::window::WindowId;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::SizeInfo;
use glutin::event_loop::EventLoopWindowTarget;
use std::sync::Arc;
use log::info;
use std::collections::HashMap;
use glutin::event_loop::EventLoop as GlutinEventLoop;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::event::Event;

use crate::display::Error;
use crate::event::{EventProxy};
use crate::display::Display;
use crate::config::Config;
use crate::term_tabs::TermTabCollection;

pub struct DisplayContextMap {
  active_window_id: Option<WindowId>,
  map: HashMap<WindowId, DisplayContext>,
  estimated_dpr: f64,
  pending_create_display: bool,
  // TODO maybe move to display
  pending_window_to_activate: Option<WindowId>,
  pending_exit: Option<WindowId>
}

impl DisplayContextMap {
  pub fn new() -> DisplayContextMap {
    DisplayContextMap {
      active_window_id: None,
      estimated_dpr: 0.0,
      map: HashMap::new(),
      pending_create_display: false,
      pending_window_to_activate: None,
      pending_exit: None,
    }
  }

  pub fn initialize(&mut self, 
    config: &Config, 
    window_event_loop: &GlutinEventLoop<Event>, 
    event_proxy: &EventProxy
  ) -> Result<(), Error> {
    // Init the estimated dpr
    self.estimated_dpr = window_event_loop
      .available_monitors()
      .next()
      .map(|m| m.hidpi_factor()).unwrap_or(1.);

    // Create the initial display
    let display_context = DisplayContext::new(self.estimated_dpr, config, window_event_loop, event_proxy)?;
    let window_id = display_context.window_id;
    self.map.insert(window_id, display_context);
    self.active_window_id = Some(window_id);

    Ok(())
  }

  pub fn is_empty(&self) -> bool {
    self.map.is_empty()
  }

  pub fn is_pending_create_display(&self) -> bool {
    self.pending_create_display
  }

  pub fn has_active_display(&mut self) -> bool{
    self.active_window_id != None
  }

  pub fn exit(&mut self, window_id: WindowId) {
    self.pending_exit = Some(window_id);

    if self.active_window_id.unwrap() == window_id {
      self.active_window_id = None;
    }

    self.map.remove_entry(&window_id);
  }

  pub fn activate_window(&mut self, window_id: WindowId) {
    self.active_window_id = Some(window_id);
  }

  pub fn deactivate_window(&mut self, window_id: WindowId) {
    if self.active_window_id != None && self.active_window_id.unwrap() == window_id {
      self.active_window_id = None;
    }
  }

  pub fn push_display_context(&mut self) {
    self.pending_create_display = true;
  }

  pub fn get_active_display_context(&self) -> &DisplayContext {
    let window_id = &self.active_window_id.unwrap();
    &self.map[window_id]
  }

  pub fn commit_changes(&mut self, 
    display_command_queue: &mut DisplayCommandQueue,
    size_info: SizeInfo,
    current_term_tab_collection: &mut TermTabCollection<EventProxy>,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<bool, Error> {
    // Handle Exit
    let did_exit = self.pending_exit != None;
    if did_exit {
      self.pending_exit = None;
    }
    
    // Handle Window Creation
    if self.pending_create_display {
      
    }

    // Handle Window Activation
    let did_activate_screen = self.pending_window_to_activate != None;    
    if did_activate_screen {
      self.active_window_id = self.pending_window_to_activate;
      self.pending_window_to_activate = None;      
    }

    // Commit any changes to the tab collection
    let is_tab_collection_dirty = current_term_tab_collection.commit_changes(
      config, 
      size_info,
    );

    // Drain the display command queue
    for command in display_command_queue.iterator() {
      match command {
        CreateDisplay => self.command_create_new_display(current_term_tab_collection, config, window_event_loop, event_proxy)?,
        _ => {}
      }
    }

    Ok(did_exit || did_activate_screen || is_tab_collection_dirty)
  }

  fn command_create_new_display(&mut self, 
    current_term_tab_collection: &mut TermTabCollection<EventProxy>,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<(), Error> {
    info!("command_create_new_display");
    let display_context = DisplayContext::new(
      self.estimated_dpr, 
      config, 
      window_event_loop, 
      event_proxy
    )?;

    let window_id = display_context.window_id;
    self.map.insert(window_id, display_context);
    self.active_window_id = Some(window_id);
    self.pending_create_display = false;

    Ok(())
  }
}

pub struct DisplayContext {
  pub window_id: WindowId,
  pub display: Arc<FairMutex<Display>>,
  pub term_tab_collection: Arc<FairMutex<TermTabCollection<EventProxy>>>,
}

impl DisplayContext {
  fn new(
    estimated_dpr: f64,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>,
    event_proxy: &EventProxy
  ) -> Result<DisplayContext, Error> {
    // Create a terminal tab collection
    // 
    // The tab collection is a collection of TerminalTab that holds the state of all tabs
    let mut term_tab_collection = TermTabCollection::new(event_proxy.clone());
    term_tab_collection.initialize(&config);
    
    // Create a display
    //
    // The display manages a window and can draw the terminal.
    let display = Display::new(config, estimated_dpr, window_event_loop)?;
    info!("PTY Dimensions: {:?} x {:?}", display.size_info.lines(), display.size_info.cols());

    // Now we can resize the terminal
    let term_tab_collection = Arc::new(FairMutex::new(term_tab_collection));
    let active_tab_arc = term_tab_collection.lock().get_active_tab().clone();
    let term_arc = active_tab_arc.lock().terminal.clone();
    let mut term = term_arc.lock();
    term.resize(&display.size_info);
    term.dirty = true;

    Ok(DisplayContext {
      window_id: display.window.window_id(),
      display: Arc::new(FairMutex::new(display)),
      term_tab_collection: term_tab_collection.clone()
    })
  }
}


#[derive (Default)]
pub struct DisplayCommandQueue {
  queue: Vec<DisplayCommand>
}

impl DisplayCommandQueue {
  pub fn push(&mut self, command: DisplayCommand) {
    self.queue.push(command);
  }

  pub fn iterator(&self) -> Iter<DisplayCommand> {
    self.queue.iter()
  }
}

#[derive (Clone)]
pub enum DisplayCommand {
  CreateDisplay,
}