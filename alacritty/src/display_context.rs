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

#[derive (Clone, PartialEq)]
pub enum DisplayCommand {
  ActivateWindow(WindowId),
  DeactivateWindow(WindowId),
  CloseWindow(WindowId),
  CreateDisplay,
  CreateTab,
  ActivateTab(usize), // tab_id
  CloseCurrentTab,
  CloseTab(usize),// tab_id
}

#[derive (Clone)]
pub enum DisplayCommandResult {
  Exit,
  Poll,
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
}

pub struct DisplayContextMap {
  active_window_id: Option<WindowId>,
  map: HashMap<WindowId, DisplayContext>,
  estimated_dpr: f64,
}

impl DisplayContextMap {
  pub fn new() -> DisplayContextMap {
    DisplayContextMap {
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
    let display_context = DisplayContext::new(self.estimated_dpr, config, window_event_loop, event_proxy)?;
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

  pub fn get_active_display_context(&self) -> &DisplayContext {
    let window_id = &self.active_window_id.unwrap();
    &self.map[window_id]
  }

  pub fn run_window_state_commands(&mut self, display_command_queue: &mut DisplayCommandQueue) {
    for command in display_command_queue.iterator() {
      match command {
        DisplayCommand::ActivateWindow(window_id) => self.command_activate_window(window_id),
        DisplayCommand::DeactivateWindow(window_id) => self.command_deactivate_window(window_id),
        DisplayCommand::CloseWindow(window_id) => self.command_close_window(window_id),
        _ => { }
      }
    }
  }

  pub fn run_user_input_commands(&mut self, 
    display_command_queue: &mut DisplayCommandQueue,
    size_info: SizeInfo,
    current_term_tab_collection: &mut TermTabCollection<EventProxy>,
    config: &Config, 
    window_event_loop: &EventLoopWindowTarget<Event>, 
    event_proxy: &EventProxy
  ) -> Result<bool, Error> {
    // Drain the display command queue
    let mut is_dirty = false;

    for command in display_command_queue.iterator() {
      let mut did_run_command = true;

      match command {
        DisplayCommand::CreateDisplay => self.command_create_new_display(config, window_event_loop, event_proxy)?,
        DisplayCommand::CreateTab => self.command_create_new_tab(current_term_tab_collection),
        DisplayCommand::ActivateTab(tab_id) => self.command_activate_tab(*tab_id, current_term_tab_collection),
        DisplayCommand::CloseCurrentTab => self.command_close_current_tab(current_term_tab_collection),
        DisplayCommand::CloseTab(tab_id) => self.command_close_tab(*tab_id, current_term_tab_collection),
        _ => { did_run_command = false }
      }

      if did_run_command {
        is_dirty = true;
      }
    }

    // Commit any changes to the tab collection
    let is_tab_collection_dirty = current_term_tab_collection.commit_changes(
      config, 
      size_info,
    );

    Ok(is_dirty || is_tab_collection_dirty)
  }

  fn command_activate_window(&mut self, window_id: &WindowId) {
    self.active_window_id = Some(*window_id);
  }

  fn command_deactivate_window(&mut self, window_id: &WindowId) {
    if self.active_window_id != None && self.active_window_id.unwrap() == *window_id {
      self.active_window_id = None;
    }
  }
  
  fn command_close_window(&mut self, window_id: &WindowId) {
    let display_ctx = self.get_active_display_context();
    let display_arc = display_ctx.display.clone();
    let display = display_arc.lock();
    let window = &display.window;
    window.close();
    
    if self.active_window_id.unwrap() == *window_id {
      self.active_window_id = None;
    }

    self.map.remove_entry(window_id);
  }

  fn command_create_new_display(&mut self, 
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

    Ok(())
  }

  fn command_create_new_tab(&mut self, tab_collection: &mut TermTabCollection<EventProxy>) {
    // TODO may be we need a window_id here as well
    info!("command_create_new_tab");
    tab_collection.push_tab();
  }

  fn command_activate_tab(&mut self, tab_id: usize, tab_collection: &mut TermTabCollection<EventProxy>) {
    info!("command_activate_tab_id tab_id: {}", tab_id);
    tab_collection.activate_tab(tab_id);
  }

  fn command_close_current_tab(&mut self, tab_collection: &mut TermTabCollection<EventProxy>) {
    info!("command_close_current_tab");
    tab_collection.close_current_tab();
  }

  fn command_close_tab(&mut self, tab_id: usize, tab_collection: &mut TermTabCollection<EventProxy>) {
    info!("command_close_tab tab_id: {}", tab_id);
    tab_collection.close_tab(tab_id);
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
