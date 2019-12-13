use crate::multi_window::term_tab::MultiWindowEvent;
use mio_extras::channel::Sender;
use std::collections::hash_map::Values;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;

use glutin::event_loop::EventLoopWindowTarget;
use glutin::window::WindowId;

use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;

use crate::config::Config;
use crate::display::Display;
use crate::display::Error;
use crate::event::EventProxy;
use crate::multi_window::term_tab::TermTab;
use crate::multi_window::term_tab_collection::TermTabCollection;

pub struct WindowContextTracker {
    active_window_id: Option<WindowId>,
    map: HashMap<WindowId, WindowContext>,
    estimated_dpr: f64,
}

impl WindowContextTracker {
    pub fn new() -> WindowContextTracker {
        WindowContextTracker { active_window_id: None, estimated_dpr: 0.0, map: HashMap::new() }
    }

    pub fn initialize(
        &mut self,
        config: &Config,
        window_event_loop: &GlutinEventLoop<Event>,
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<(), Error> {
        // Init the estimated dpr
        self.estimated_dpr =
            window_event_loop.available_monitors().next().map(|m| m.hidpi_factor()).unwrap_or(1.);

        // Create the initial display
        let display_context = WindowContext::new(self.estimated_dpr, config, window_event_loop, event_proxy, dispatcher)?;
        let window_id = display_context.window_id;
        self.map.insert(window_id, display_context);
        self.active_window_id = Some(window_id);

        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn has_active_window(&mut self) -> bool {
        self.active_window_id != None
    }

    pub fn get_all_window_contexts(&self) -> Values<WindowId, WindowContext> {
        self.map.values().clone()
    }

    pub fn get_active_window_context(&self) -> WindowContext {
        if self.active_window_id == None { 
            panic!("window_context_tracker get_active_window_context called on empty collection") 
        }

        let window_id = &self.active_window_id.unwrap();
        self.map[window_id].clone()
    }

    pub fn get_context(&self, window_id: WindowId) -> Option<WindowContext> {
        if self.map.contains_key(&window_id) {
            return Some(self.map[&window_id].clone());
        }

        None
    }

    pub(super) fn activate_window(&mut self, window_id: WindowId) {
        self.active_window_id = Some(window_id);
        self.get_active_window_context().display.lock().request_resize();
    }

    pub(super) fn deactivate_window(&mut self, window_id: WindowId) {
        if self.active_window_id != None && self.active_window_id.unwrap() == window_id {
            self.active_window_id = None;
        }
    }

    pub(super) fn close_window(&mut self, window_id: WindowId) {
        if !self.map.contains_key(&window_id) {
            return;
        }

        if self.active_window_id != None && self.active_window_id.unwrap() == window_id {
            self.active_window_id = None;
        }

        let (_, window_ctx) = self.map.remove_entry(&window_id).unwrap();
        let display_arc = window_ctx.display.clone();
        let display = display_arc.lock();
        display.window.close();
    }

    pub(super) fn create_window_context(
        &mut self,
        config: &Config,
        window_event_loop: &EventLoopWindowTarget<Event>,
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<(), Error> {
        info!("command_create_new_display");
        let display_context = WindowContext::new(
            self.estimated_dpr, 
            config,
            window_event_loop,
            event_proxy,
            dispatcher
        )?;

        let window_id = display_context.window_id;
        self.map.insert(window_id, display_context);
        self.active_window_id = Some(window_id);
        
        Ok(())
    }
}

#[derive(Clone)]
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
        event_proxy: &EventProxy,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> Result<WindowContext, Error> {
        // Create a terminal tab collection
        //
        // The tab collection is a collection of TerminalTab that holds the state of all tabs
        let mut term_tab_collection = TermTabCollection::new(event_proxy.clone());
        let mut active_tab = term_tab_collection.initialize(&config, dispatcher);

        // Create a display
        //
        // The display manages a window and can draw the terminal.
        let mut display = Display::new(config, estimated_dpr, window_event_loop)?;
        let window_id = display.window.window_id();
        active_tab.set_window_id(window_id);
        info!("PTY Dimensions: {:?} x {:?}", display.size_info.lines(), display.size_info.cols());

        // Handle Cascading on mac os 
        #[cfg(target_os = "macos")]
        WindowContext::handle_macos_window_cascading();

        // Sync Size of the terminal and display
        display.request_resize();

        Ok(WindowContext {
            window_id: display.window.window_id(),
            display: Arc::new(FairMutex::new(display)),
            term_tab_collection: Arc::new(FairMutex::new(term_tab_collection)),
        })
    }

    pub fn get_active_tab(&self) -> Option<TermTab<EventProxy>> {
        let tab_collection = self.term_tab_collection.lock();
        tab_collection.get_active_tab()
    }

    #[cfg(target_os = "macos")]
    fn handle_macos_window_cascading() {
        use objc::{ msg_send, sel, sel_impl };
        use cocoa::{ base::{id, nil}, foundation::{NSPoint}};

        unsafe {
            let shared_application = cocoa::appkit::NSApplication::sharedApplication(nil);
            let windows: id = msg_send![shared_application,  windows];

            let main_window: id = msg_send![shared_application,  mainWindow];
            let ns_point: NSPoint = msg_send![main_window, cascadeTopLeftFromPoint: NSPoint {x: 0.0, y: 0.0}];

            let window: id = msg_send![windows, lastObject];
            let _result: id = msg_send![window, cascadeTopLeftFromPoint: ns_point];   
        }

    }
}
