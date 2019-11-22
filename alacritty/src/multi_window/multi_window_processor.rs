use crate::event::EventProxy;
use crate::config::Config;
use glutin::event_loop::{ControlFlow};

use alacritty_terminal::message_bar::MessageBuffer;

use crate::event::{Processor};
use crate::multi_window::window_context_tracker::WindowContextTracker;
use crate::multi_window::command_queue::{MultiWindowCommandQueue, MultiWindowCommandResult};

use alacritty_terminal::event::Event;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;

#[derive (Default)]
pub struct MultiWindowProcessor {}

impl MultiWindowProcessor {
  pub fn run(&self, 
    mut config: Config,
    mut window_event_loop: GlutinEventLoop<Event>,
    mut window_context_tracker: WindowContextTracker, 
    event_proxy: EventProxy,
  ) {
    // Setup shared storage for message UI
    let message_buffer = MessageBuffer::new();

    // Shared User Input Event processor
    //
    // Need the Rc<RefCell<_>> here since a ref is shared in the resize callback
    let mut processor = Processor::new(
        message_buffer,
        config.font.size,
    );
    
    // Event queue 
    //
    //
    let mut event_queue = Vec::new();

    window_event_loop.run_return(|event, _event_loop, mut control_flow| {    
        let mut multi_window_queue = MultiWindowCommandQueue::default();

        // Activation & Deactivation of windows           
        match multi_window_queue.handle_multi_window_events(&mut window_context_tracker, &event) {
            MultiWindowCommandResult::RestartLoop => return,
            MultiWindowCommandResult::Exit => {
                *control_flow = ControlFlow::Exit;
                return;
            },
            _ => {}
        }

        if !window_context_tracker.has_active_display() { return; }

        // Process events for the active display, user input etc.
        let mut window_ctx = window_context_tracker.get_active_display_context();

        processor.run(
            &mut event_queue,
            &mut multi_window_queue, 
            &mut window_ctx,
            event,
            &mut control_flow,
            &mut config,
        );

        // Process windows specific events
        match multi_window_queue.run_user_input_commands(
            &mut window_context_tracker,
            &mut window_ctx,
            &config,
            _event_loop,
            &event_proxy,
        ) {
          Ok(_) => {}
          Err(_err) => { }
        };

        // Draw the inactive windows
    });
  }
}