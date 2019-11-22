use mio_extras::channel::Sender;
use std::sync::Arc;

use alacritty_terminal::clipboard::Clipboard;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::event::OnResize;
use alacritty_terminal::event_loop::EventLoop;
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::SizeInfo;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty;

use crate::config::Config;

#[derive(Clone)]
pub struct TermTab<T> {
    pub terminal: Arc<FairMutex<Term<T>>>,
    pub resize_handle: Arc<FairMutex<Box<dyn OnResize>>>,
    pub loop_tx: Sender<Msg>,
    // pub io_thread: JoinHandle<(EventLoop, terminal_event_loop::State)>,
}

impl<'a, T: 'static + 'a + EventListener + Clone + Send> TermTab<T> {
    pub(super) fn new(config: &Config, display_size_info: SizeInfo, event_proxy: T) -> TermTab<T> {
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
        let resize_handle = pty.resize_handle();
        #[cfg(not(windows))]
        let resize_handle = pty.fd.as_raw_fd();

        // Create the pseudoterminal I/O loop
        //
        // pty I/O is ran on another thread as to not occupy cycles used by the
        // renderer and input processing. Note that access to the terminal state is
        // synchronized since the I/O loop updates the state, and the display
        // consumes it periodically.
        let terminal_event_loop =
            EventLoop::new(terminal.clone(), event_proxy.clone(), pty, config);

        // The event loop channel allows write requests from the event processor
        // to be sent to the pty loop and ultimately written to the pty.
        let loop_tx = terminal_event_loop.channel();

        // Kick off the I/O thread
        // TODO keep the list of threads for later cleanup
        //let io_thread =
        terminal_event_loop.spawn();

        TermTab {
            terminal,
            resize_handle: Arc::new(FairMutex::new(Box::new(resize_handle))),
            loop_tx: loop_tx.clone(),
            //io_thread,
        }
    }
}
