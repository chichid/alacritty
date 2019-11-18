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

pub struct TermTabCollection<T> {
    event_proxy: T,
    active_term: usize,
    term_collection: Vec<Arc<FairMutex<TermTab<T>>>>,
    pending_add_term: usize,
    pending_active_term: usize,
}

impl<'a, T: 'static + Clone + Send + EventListener> TermTabCollection<T> {
    pub fn new(event_proxy: T) -> TermTabCollection<T> {
        TermTabCollection {
            event_proxy: event_proxy.clone(),
            active_term: 0,
            term_collection: Vec::new(),
            pending_add_term: 0,
            pending_active_term: 0
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
        self.push_term_tab();
        self.activate_term_tab(0);
        self.commit_changes(config, dummy_display_size_info);
    }
    
    pub fn get_active_term(&self) -> &Arc<FairMutex<TermTab<T>>> {
        &self.term_collection[self.active_term]
    }

    pub fn activate_term_tab(&mut self, term_id: usize) {
        self.pending_active_term = term_id;
    }

    pub fn push_term_tab(&mut self) -> usize {
        let new_term_id = self.term_collection.len() + self.pending_add_term;        
        self.pending_add_term += 1;

        self.activate_term_tab(new_term_id);

        new_term_id
    }

    pub fn commit_changes(&mut self, config: &Config, size_info: SizeInfo) -> bool {
        // Add new terminals
        let mut is_dirty = false;

        for _ in 0..self.pending_add_term {
            let term_context = TermTab::new(config, size_info, self.event_proxy.clone());
            self.term_collection.push(Arc::new(FairMutex::new(term_context)));
            is_dirty = true;
        }

        self.pending_add_term = 0;

        // Activate the terminal id needed
        if self.pending_active_term != self.active_term && self.pending_active_term < self.term_collection.len() {
            println!("Activitng term {:?}", self.pending_active_term);
            self.active_term = self.pending_active_term;
            is_dirty = true;
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