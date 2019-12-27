use std::sync::Arc;
use glutin::window::WindowId;
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
  fn new(term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>) -> TabBarState<T> {
    TabBarState {
      current_dragging_tab: None,
      current_drop_zone: None,
      term_tab_collection,
    }
  }

}

pub struct TabBarProcessor<'a, T> {
  tab_bar_state: &'a mut TabBarState<T>,
}

impl<'a, T> TabBarProcessor<'a, T> {
  fn new(tab_bar_state: &'a mut TabBarState<T>) -> TabBarProcessor<'a, T> {
    TabBarProcessor {
      tab_bar_state
    }
  }

  pub(super) fn handle_mouse_events(&self) {
    // TODO Implement
  }
}

pub struct TabBarRenderer<'a, T> {
  tab_bar_state: &'a mut TabBarState<T>,
}

impl<'a, T> TabBarRenderer<'a, T> {
  fn new(tab_bar_state: &'a mut TabBarState<T>) -> TabBarRenderer<'a, T> {
    TabBarRenderer {
      tab_bar_state
    }
  }

  fn render_tabs(&self) {

  }
}