use glutin::window::WindowId;
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

use crate::multi_window::term_tab::TermTab;
use crate::multi_window::term_tab_collection::TermTabCollection;
use crate::multi_window::command_queue::{DisplayCommand, DisplayCommandQueue};

pub struct WindowContextTracker {
  active_window_id: Option<WindowId>,
  map: HashMap<WindowId, WindowContext>,
  estimated_dpr: f64,
}

impl WindowContextTracker {
  pub fn new() -> WindowContextTracker {
    WindowContextTracker {
      active_window_id: None,
      estimated_dpr: 0.0,
      map: HashMap::new(),
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
    let display_context = WindowContext::new(self.estimated_dpr, config, window_event_loop, event_proxy)?;
    let window_id = display_context.window_id;
    self.map.insert(window_id, display_context);
    self.active_window_id = Some(window_id);

    Ok(())
  }

  pub fn is_empty(&self) -> bool {
    self.map.is_empty()
  }

  pub fn has_active_display(&mut self) -> bool{
    self.active_window_id != None
  }

  pub fn get_active_display_context(&self) -> WindowContext {
    let window_id = &self.active_window_id.unwrap();
    self.map[window_id].clone()
  }

  pub(super) fn activate_window(&mut self, window_id: WindowId) {
    self.active_window_id = Some(window_id);
  }

  pub(super) fn deactivate_window(&mut self, window_id: WindowId) {
    if self.active_window_id != None && self.active_window_id.unwrap() == window_id {
      self.active_window_id = None;
    }
  }
  
  pub(super) fn close_window(&mut self, window_id: WindowId) {
    let display_ctx = self.get_active_display_context();
    let display_arc = display_ctx.display.clone();
    let display = display_arc.lock();
    let window = &display.window;
    window.close();
    
    if self.active_window_id.unwrap() == window_id {
      self.active_window_id = None;
    }

    self.map.remove_entry(&window_id);
  }

  pub(super) fn create_display(&mut self, 
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<(), Error> {
    info!("command_create_new_display");
    let display_context = WindowContext::new(
      self.estimated_dpr, 
      config, 
      window_event_loop, 
      event_proxy
    )?;

    let window_id = display_context.window_id;
    self.map.insert(window_id, display_context);
    self.active_window_id = Some(window_id);

    Ok(())
  }
}

#[derive (Clone)]
pub struct WindowContext {
  pub window_id: WindowId,
  pub display: Arc<FairMutex<Display>>,
  pub term_tab_collection: Arc<FairMutex<TermTabCollection<EventProxy>>>,
}

impl WindowContext {
  fn new(
    estimated_dpr: f64,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>,
    event_proxy: &EventProxy
  ) -> Result<WindowContext, Error> {
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
    let active_tab = term_tab_collection.lock().get_active_tab().clone();
    let term_arc = active_tab.terminal.clone();
    let mut term = term_arc.lock();
    term.resize(&display.size_info);
    term.dirty = true;

    Ok(WindowContext {
      window_id: display.window.window_id(),
      display: Arc::new(FairMutex::new(display)),
      term_tab_collection: term_tab_collection.clone()
    })
  }

  pub fn get_active_tab(&self) -> TermTab<EventProxy> {
    let tab_collection = self.term_tab_collection.lock();
    tab_collection.get_active_tab().clone()
  }
}
