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
  pending_window_to_activate: Option<WindowId>,
}

impl DisplayContextMap {
  pub fn new() -> DisplayContextMap {
    DisplayContextMap {
      active_window_id: None,
      estimated_dpr: 0.0,
      map: HashMap::new(),
      pending_create_display: false,
      pending_window_to_activate: None,
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

  pub fn push_display_context(&mut self) {
    self.pending_create_display = true;
  }

  pub fn activate_window(&mut self, window_id: WindowId) {
    println!("Activate window called {:?}", window_id);
    self.pending_window_to_activate = Some(window_id);
  }

  pub fn get_display_context(&self) -> &DisplayContext {
    let win_id = &self.active_window_id.unwrap();
    &self.map[win_id]
  }

  pub fn commit_changes<T: 'static +  EventListener + Clone + Send>(&mut self, 
    size_info: SizeInfo,
    current_term_tab_collection: &mut TermTabCollection<T>,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<bool, Error> {
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

    Ok(did_activate_screen || is_tab_collection_dirty)
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

    Ok(DisplayContext {
      window_id: display.window.window_id(),
      display: Arc::new(FairMutex::new(display)),
      term_tab_collection: Arc::new(FairMutex::new(term_tab_collection))
    })
  }
}
