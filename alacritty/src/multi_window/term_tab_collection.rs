use mio_extras::channel::Sender;
use crate::multi_window::term_tab::MultiWindowEvent;
use glutin::window::WindowId;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::SizeInfo;

use crate::config::Config;
use crate::multi_window::term_tab::TermTab;

#[cfg(not(windows))]
use std::os::unix::io::AsRawFd;

pub struct TermTabCollection<T> {
    event_proxy: T,
    active_tab: usize,
    tab_collection: Vec<TermTab<T>>,
}

impl<'a, T: 'static + Clone + Send + EventListener> TermTabCollection<T> {
    pub fn get_active_tab(&self) -> TermTab<T> {
        self.tab_collection[self.active_tab].clone()
    }

    pub(super) fn new(event_proxy: T) -> TermTabCollection<T> {
        TermTabCollection {
            event_proxy: event_proxy.clone(),
            active_tab: 0,
            tab_collection: Vec::new(),
        }
    }

    pub(super) fn initialize(&mut self, config: &Config, dispatcher: Sender<MultiWindowEvent>) {
        // This decouples the terminal initialization from the display, to allow faster startup time
        // For the first terminal, the resizing in the event loop kicks in and will eventually
        // resize the current terminal and value here will do
        let dummy_display_size_info = SizeInfo {
            width: 100.0,
            height: 100.0,
            cell_width: 1.0,
            cell_height: 1.0,
            padding_x: 0.0,
            padding_y: 0.0,
            dpr: 1.0,
        };

        // Add the intiial terminal
        // 
        // The window_id will be pushed to the terminal when the display is created later
        // the size_info as well will be updated when the display is created
        self.add_tab(config, dummy_display_size_info, None, &dispatcher);
        self.activate_tab(0);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tab_collection.is_empty()
    }

    pub(super) fn add_tab(&mut self,
        config: &Config,
        size_info: SizeInfo,
        window_id: Option<WindowId>, 
        dispatcher: &Sender<MultiWindowEvent>,
    ) -> usize {
        let tab_id = self.tab_collection.len();
        let new_tab = TermTab::new(window_id, tab_id, dispatcher.clone(), config, size_info, self.event_proxy.clone());
        self.tab_collection.push(new_tab);

        tab_id
    }

    pub(super) fn activate_tab(&mut self, tab_id: usize) {
        if tab_id < self.tab_collection.len() {
            self.active_tab = tab_id;
        }
    }

    pub(super) fn close_current_tab(&mut self) {
        self.close_tab(self.active_tab);        
    }

    pub(super) fn close_tab(&mut self, tab_id: usize) {
        self.tab_collection.remove(tab_id);

        if self.active_tab >= self.tab_collection.len() && self.active_tab != 0 {
            self.active_tab = self.tab_collection.len() - 1;
        }

        // Update tab_ids
        for (tab_id, tab) in self.tab_collection.iter_mut().enumerate() {
            tab.update_tab_id(tab_id);
        }
    }
}
