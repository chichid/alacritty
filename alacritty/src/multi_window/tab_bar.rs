use std::sync::Arc;

use glutin::event::Event as GlutinEvent;
use glutin::window::WindowId;

use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;

use crate::multi_window::term_tab_collection::TermTabCollection;

pub struct DropZone {
  x: f32,
  y: f32,
  width: f32,
  height: f32,
  window_id: WindowId,
}

pub struct TabBarState<T> {
  pub current_dragging_tab: Option<usize>,
  pub current_drop_zone: Option<DropZone>,
  term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>,
}

impl<T> TabBarState<T> {
  pub(super) fn new(term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>) -> TabBarState<T> {
    TabBarState { current_dragging_tab: None, current_drop_zone: None, term_tab_collection }
  }
}

pub(super) struct TabBarProcessor<T> {
  tab_bar_state: Arc<FairMutex<TabBarState<T>>>,
}

impl<T> TabBarProcessor<T> {
  pub(super) fn new(tab_bar_state: Arc<FairMutex<TabBarState<T>>>) -> TabBarProcessor<T> {
    TabBarProcessor { tab_bar_state }
  }

  pub(super) fn handle_event(&self, event: GlutinEvent<Event>) {
    if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
        use glutin::event::WindowEvent::*;

        match event {
          MouseInput { state, button, .. } => {
            println!("Mouse input {:?} {:?}", state, button);
          }

          CursorMoved { position: lpos, .. } => {
            println!("Cursor moving {:?}", lpos);
          }

          CursorEntered { .. } => {
            println!("Cursor Entered window {:?}", window_id);
          }

          CursorLeft { .. } => {
            println!("Cursor Left Window {:?}", window_id);
          }

          _ => {}
        }
    }
  }
}

pub struct TabBarRenderer<T> {
  tab_bar_state: Arc<FairMutex<TabBarState<T>>>,
}

impl<T> TabBarRenderer<T> {
  pub fn new(tab_bar_state: Arc<FairMutex<TabBarState<T>>>) -> TabBarRenderer<T> {
    TabBarRenderer { tab_bar_state }
  }

  fn render(&self, tab_bar_state: &mut TabBarState<T>) {}
}
