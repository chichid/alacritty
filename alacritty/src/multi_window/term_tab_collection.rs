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
    term_collection: Vec<TermTab<T>>,
    pending_tab_activate: usize,
    pending_commit_delete_tab: bool,
}

impl<'a, T: 'static + Clone + Send + EventListener> TermTabCollection<T> {
    pub fn get_active_tab(&self) -> TermTab<T> {
        self.term_collection[self.active_tab].clone()
    }

    pub(super) fn new(event_proxy: T) -> TermTabCollection<T> {
        TermTabCollection {
            event_proxy: event_proxy.clone(),
            active_tab: 0,
            term_collection: Vec::new(),
            pending_tab_to_add: 0,
            pending_tab_activate: 0,
            pending_commit_delete_tab: false,
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
        self.add_tab(config, dummy_display_size_info, None, &dispatcher);
        self.activate_tab(0);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.term_collection.is_empty()
    }

    pub(super) fn activate_tab(&mut self, tab_id: usize) {
        self.pending_tab_activate = tab_id;
    }

    pub(super) fn close_current_tab(&mut self) {
        self.close_tab(self.active_tab);
        self.activate_tab(self.active_tab);
    }

    pub(super) fn close_tab(&mut self, tab_id: usize) -> bool {
        self.term_collection.remove(tab_id);

        if self.active_tab >= self.term_collection.len() && self.active_tab != 0 {
            self.active_tab = self.term_collection.len() - 1;
        }

        self.pending_commit_delete_tab = true;

        self.term_collection.is_empty()
    }

    pub(super) fn add_tab(&mut self,
        config: &Config,
        size_info: SizeInfo,
        window_id: Option<WindowId>, 
        dispatcher: &Sender<MultiWindowEvent>,
    ) -> usize {
        let tab_id = self.term_collection.len();
        let new_tab = TermTab::new(window_id, tab_id, dispatcher.clone(), config, size_info, self.event_proxy.clone());
        self.term_collection.push(new_tab);

        tab_id
    }

    pub(super) fn commit_changes(
        &mut self, 
        window_id: Option<WindowId>,
        config: &Config,
        size_info: SizeInfo,
        dispatcher: Sender<MultiWindowEvent>,
    ) -> bool {
        // Add new terminals
        let mut is_dirty = false;

        // Activate the terminal id needed
        if self.pending_tab_activate != self.active_tab
            && self.pending_tab_activate < self.term_collection.len()
        {
            self.active_tab = self.pending_tab_activate;
            is_dirty = true;
        }

        // Commit delete changes
        if self.pending_commit_delete_tab {
            is_dirty = true;
            self.pending_commit_delete_tab = false;
        }

        is_dirty
    }
}
