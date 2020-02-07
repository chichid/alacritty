use alacritty_terminal::event_loop::Notifier;
use glutin::window::WindowId;
use mio_extras::channel::{self, Receiver};

use alacritty_terminal::event::Event;
use glutin::event::Event as GlutinEvent;
use glutin::event_loop::ControlFlow;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::platform::desktop::EventLoopExtDesktop;

use crate::config::Config;
use crate::display::Error as DisplayError;
use crate::event::EventProxy;
use crate::multi_window::command_queue::{MultiWindowCommandQueue, MultiWindowCommand};
use crate::multi_window::term_tab::MultiWindowEvent;
use crate::multi_window::window_context_tracker::WindowContextTracker;

#[derive(Default)]
pub struct MultiWindowProcessor {}

impl MultiWindowProcessor {
  pub fn run(
    &self,
    mut config: Config,
    mut window_event_loop: GlutinEventLoop<Event>,
    event_proxy: EventProxy,
  ) -> Result<(), DisplayError> {
    let mut event_queue = Vec::new();
    let (multi_window_tx, multi_window_rx) = channel::channel();

    let mut window_context_tracker = WindowContextTracker::new();
    window_context_tracker.initialize(
      &mut config,
      &window_event_loop,
      &event_proxy,
      multi_window_tx.clone(),
    )?;

    // Run the process event loop
    window_event_loop.run_return(move |event, event_loop, mut control_flow| {
      let mut command_queue = MultiWindowCommandQueue::default();

      // Activation, Deactivation and Closing of windows
      self.handle_multi_window_events(
        &mut command_queue,
        event.clone(),
        &mut control_flow,
      );

      // PTY Detach for all windows and dirty state for inactive terminals
      self.handle_pty_events(
        &mut command_queue,
        &multi_window_rx
      );

      // Run Window Events immediately
      if let Err(error) = command_queue.run(
        &mut window_context_tracker,
        &config,
        &event_loop,
        &event_proxy,
        multi_window_tx.clone(),
      ) {
        // TODO log error
      }

      // If we closed all the windows, we're done
      if window_context_tracker.is_empty() {
        *control_flow = ControlFlow::Exit;
        return;
      }

      // If nothing is active, process the inactive windows
      // otherwise process the active window first, then draw the inactive windows
      if !window_context_tracker.has_active_window() {
        self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
        return;
      }

      let active_ctx = window_context_tracker.get_active_window_context();
      let active_tab = active_ctx.active_tab();
      if active_tab.is_none() {
        self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
        return;
      };

      // TODO move to the command_queue
      // Handle Tab-bar events
      let (need_redraw, skip_processor_run, cursor_icon) = {
        let size_info = active_ctx.processor.lock().get_size_info();
      
        active_ctx.tab_bar_processor.lock().handle_event(
          &active_ctx.term_tab_collection.lock(),
          &config,
          &size_info,
          event.clone(),
          &mut command_queue,
        )
      };

      if let Some(cursor_icon) = cursor_icon {
        active_ctx.processor.lock().window_mut().set_mouse_cursor(cursor_icon);
      }

      if need_redraw {
        active_ctx.processor.lock().request_redraw();
      }

      if let Err(error) = command_queue.run(
        &mut window_context_tracker,
        &config,
        &event_loop,
        &event_proxy,
        multi_window_tx.clone(),
      ) {
        // TODO LOG Error
      };

      // Processor
      if !skip_processor_run {
        let mut processor = active_ctx.processor.lock();
        let active_tab = active_tab.unwrap();
        let mut notifier = Notifier(active_tab.loop_tx.clone());

        processor.make_current();

        processor.run_iteration(
          &mut notifier,
          &mut event_queue,
          event,
          &mut control_flow,
          active_tab.terminal,
          &mut config,
          &mut command_queue,
        );
      }

      if let Err(error) = command_queue.run(
        &mut window_context_tracker,
        &config,
        &event_loop,
        &event_proxy,
        multi_window_tx.clone(),
      ) {
       // TODO LOG Error 
      }

      // Handle windows that are visible but not active
      self.draw_inactive_visible_windows(&config, &mut window_context_tracker);
    });

    Ok(())
  }

  fn handle_pty_events(
    &self,
    command_queue: &mut MultiWindowCommandQueue,
    receiver: &Receiver<MultiWindowEvent>,
  ) {
    match receiver.try_recv() {
      Ok(result) => {
        if result.window_id.is_none() {
          return;
        }

        let window_id = result.window_id.unwrap();
        let tab_id = result.tab_id;
        
        match result.wrapped_event {
          Event::Exit => {
            command_queue.push(MultiWindowCommand::CloseTab(window_id, tab_id));
          }

          Event::Title(title) => {
            command_queue.push(MultiWindowCommand::SetTabTitle(window_id, tab_id, title));
          }

          _ => {}
        }
      }
      Err(err) => {
        // TODO log errors
        // change the result of this function to be Result once that's done
      }
    }
  }

  fn handle_multi_window_events(
    &self,
    command_queue: &mut MultiWindowCommandQueue,
    event: GlutinEvent<Event>,
    control_flow: &mut ControlFlow,
  ) {
    match event {
      // Process events
      GlutinEvent::EventsCleared => {
        *control_flow = ControlFlow::Wait;
      }

      // Handle Window Activate, Deactivate, Close Events
      GlutinEvent::WindowEvent { event, window_id, .. } => {
        use glutin::event::WindowEvent::*;

        match event {
          Focused(is_focused) => {
            if is_focused {
              command_queue.push(MultiWindowCommand::ActivateWindow(window_id));
            } else {
              command_queue.push(MultiWindowCommand::DeactivateWindow(window_id))
            }
          }

          CloseRequested => {
            command_queue.push(MultiWindowCommand::CloseWindow(window_id))
          }

          _ => {}
        }
      }

      _ => {}
    }
  }

  fn draw_inactive_visible_windows(
    &self,
    config: &Config,
    context_tracker: &mut WindowContextTracker,
  ) {
    let has_active_display = context_tracker.has_active_window();

    let active_window_id = if has_active_display {
      Some(context_tracker.get_active_window_context().window_id)
    } else {
      None
    };

    for inactive_ctx in context_tracker.get_all_window_contexts() {
      // TODO check if the window related to the context is maximized
      if !has_active_display || inactive_ctx.window_id != active_window_id.unwrap() {
        let tab = inactive_ctx.active_tab().unwrap();
        if tab.terminal.lock().dirty {
          tab.terminal.lock().dirty = false;
          inactive_ctx.processor.lock().redraw(tab.terminal.clone(), config);
        }
      }
    }
  }
}
