use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::event::OnResize;
use alacritty_terminal::clipboard::Clipboard;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::SizeInfo;
use alacritty_terminal::event_loop::{Notifier, EventLoop};
use alacritty_terminal::tty;

use crate::config::Config;

#[cfg(not(windows))]
use std::os::unix::io::AsRawFd;

pub struct TermTabCollection<T> {
    event_proxy: T,
    active_tab: usize,
    term_collection: Vec<Arc<FairMutex<TermTab<T>>>>,
    pending_tab_to_add: usize,
    pending_tab_activate: usize,
    pending_commit_delete_tab: bool,
}

impl<'a, T: 'static + Clone + Send + EventListener> TermTabCollection<T> {
    pub fn new(event_proxy: T) -> TermTabCollection<T> {
        TermTabCollection {
            event_proxy: event_proxy.clone(),
            active_tab: 0,
            term_collection: Vec::new(),
            pending_tab_to_add: 0,
            pending_tab_activate: 0,
            pending_commit_delete_tab: false
        }
    }

    pub fn initialize(&mut self, config: &Config) {
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
        self.push_tab();
        self.activate_tab(0);
        self.commit_changes(config, dummy_display_size_info);
    }

    pub fn is_empty(&self) -> bool {
        self.term_collection.is_empty()
    }

    pub fn get_active_tab(&self) -> &Arc<FairMutex<TermTab<T>>> {
        &self.term_collection[self.active_tab]
    }

    pub fn activate_tab(&mut self, tab_id: usize) {
        self.pending_tab_activate = tab_id;
    }

    pub fn close_all_tabs(&mut self) {
        self.term_collection.clear();
        self.pending_commit_delete_tab = true;
    }

    pub fn close_current_tab(&mut self) {
        self.close_tab(self.active_tab);
        self.activate_tab(self.active_tab);
    }

    pub fn close_tab(&mut self, tab_id: usize) -> bool {
        self.term_collection.remove(tab_id);

        if self.active_tab >= self.term_collection.len() && self.active_tab != 0 {
            self.active_tab = self.term_collection.len() - 1;
        }
        
        self.pending_commit_delete_tab = true;

        self.term_collection.is_empty()
    }

    pub fn push_tab(&mut self) -> usize {
        let new_tab_id = self.term_collection.len() + self.pending_tab_to_add;        
        self.pending_tab_to_add += 1;

        self.activate_tab(new_tab_id);

        new_tab_id
    }

    pub fn commit_changes(&mut self, config: &Config, size_info: SizeInfo) -> bool {
        // Add new terminals
        let mut is_dirty = false;

        for _ in 0..self.pending_tab_to_add {
            let term_context = TermTab::new(config, size_info, self.event_proxy.clone());
            self.term_collection.push(Arc::new(FairMutex::new(term_context)));
            is_dirty = true;
        }

        self.pending_tab_to_add = 0;

        // Activate the terminal id needed
        if self.pending_tab_activate != self.active_tab && self.pending_tab_activate < self.term_collection.len() {
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

pub struct TermTab<T> {
    pub terminal: Arc<FairMutex<Term<T>>>,   
    pub resize_handle: Box<dyn OnResize>,
    pub notifier: Box<Notifier>,
    // pub io_thread: JoinHandle<(EventLoop, terminal_event_loop::State)>,
}

impl <'a, T: 'static + 'a + EventListener + Clone + Send> TermTab<T> {
    pub fn new(config: &Config, display_size_info: SizeInfo, event_proxy: T) -> TermTab<T> {
        // Create new native clipboard
        #[cfg(not(any(target_os = "macos", windows)))]
        let clipboard = Clipboard::new(display.window.wayland_display());
        #[cfg(any(target_os = "macos", windows))]
        let clipboard = Clipboard::new();

        // Create the terminal
        //
        // This object contains all of the state about what's being displayed. It's
        // wrapped in a clonable mutex since both the I/O loop and display need to
        // access it.
        let terminal = Term::new(config, &display_size_info, clipboard, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Create the pty
        //
        // The pty forks a process to run the shell on the slave side of the
        // pseudoterminal. A file descriptor for the master side is retained for
        // reading/writing to the shell.
        #[cfg(not(any(target_os = "macos", windows)))]
        let pty = tty::new(config, &display_size_info, display.window.x11_window_id());
        #[cfg(any(target_os = "macos", windows))]
        let pty = tty::new(config, &display_size_info, None);

        // Create PTY resize handle
        //
        // This exists because rust doesn't know the interface is thread-safe
        // and we need to be able to resize the PTY from the main thread while the IO
        // thread owns the EventedRW object.
        #[cfg(windows)]
        let resize_handle = Box::new(pty.resize_handle());
        #[cfg(not(windows))]
        let resize_handle = Box::new(pty.fd.as_raw_fd());

        // Create the pseudoterminal I/O loop
        //
        // pty I/O is ran on another thread as to not occupy cycles used by the
        // renderer and input processing. Note that access to the terminal state is
        // synchronized since the I/O loop updates the state, and the display
        // consumes it periodically.
        let terminal_event_loop = EventLoop::new(terminal.clone(), event_proxy.clone(), pty, config);

        // The event loop channel allows write requests from the event processor
        // to be sent to the pty loop and ultimately written to the pty.
        let loop_tx = terminal_event_loop.channel();
        let notifier = Box::new(Notifier(loop_tx.clone()));

        // Kick off the I/O thread
        // TODO keep the list of threads for later cleanup
        //let io_thread = 
        terminal_event_loop.spawn();

        TermTab {
            terminal,
            resize_handle,
            notifier,
            //io_thread,
        }
    }
}