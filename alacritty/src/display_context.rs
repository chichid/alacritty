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
    if self.active_window_id != None { true } else { false }
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
    if (self.active_window_id != None && self.active_window_id.unwrap() == window_id) {
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

  pub fn commit_changes<T: 'static +  EventListener + Clone + Send>(&mut self, 
    size_info: SizeInfo,
    current_term_tab_collection: &mut TermTabCollection<T>,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<bool, Error> {
    // Handle Exit
    let did_exit = if self.pending_exit != None {
      let window_id = self.pending_exit.unwrap();
      
      // current_term_tab_collection.close_all_tabs();
      self.pending_exit = None;
      
      true
    } else { 
      false
    };
    
    // Handle Window Creation
    if self.pending_create_display {
      let display_context = DisplayContext::new(self.estimated_dpr, config, window_event_loop, event_proxy)?;
      let window_id = display_context.window_id;
      self.map.insert(window_id, display_context);
      self.active_window_id = Some(window_id);
      self.pending_create_display = false;
    }

    // Handle Window Activation
    let did_activate_screen = if self.pending_window_to_activate != None {
      self.active_window_id = self.pending_window_to_activate;
      self.pending_window_to_activate = None;
      true
    } else {
      false
    };

    // Commit any changes to the tab collection
    let is_tab_collection_dirty = current_term_tab_collection.commit_changes(
      config, 
      size_info,
    );

    Ok(did_exit || did_activate_screen || is_tab_collection_dirty)
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
