use glutin::window::CursorIcon;
use std::sync::Arc;

use glutin::event::Event as GlutinEvent;
use glutin::window::WindowId;
use glutin::dpi::LogicalPosition;

use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::index::Line;
use alacritty_terminal::term::color::Rgb;
use alacritty_terminal::renderer::GlyphCache;
use alacritty_terminal::term::SizeInfo;
use alacritty_terminal::renderer::QuadRenderer;
use alacritty_terminal::renderer::rects::RenderRect;

use crate::event::EventProxy;
use crate::config::Config;
use crate::multi_window::term_tab_collection::TermTabCollection;
use crate::multi_window::command_queue::{MultiWindowCommandQueue, MultiWindowCommand};

const CLOSE_ICON_PADDING: f32 = 10.0;
const CLOSE_ICON_WIDTH: f32 = 5.0;

#[derive (Clone, Copy)]
pub struct DraggedTab {
  pub tab_id: usize,
  pub x: f64,
  pub y: f64,
}

pub struct TabBarState<T> {
  pub hovered_tab: Option<usize>,
  pub dragged_tab: Option<DraggedTab>,
  term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>,
}

impl<T> TabBarState<T> {
  pub(super) fn new(term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>) -> TabBarState<T> {
    TabBarState { 
      hovered_tab: None,
      dragged_tab: None,
      term_tab_collection,
    }
  }
}

pub(super) struct TabBarProcessor {
  tab_bar_state: Arc<FairMutex<TabBarState<EventProxy>>>,
  is_mouse_down: bool,
  mouse_down_position: Option<LogicalPosition>,
  current_mouse_position: Option<LogicalPosition>,
  mouse_down_window: Option<WindowId>,
  current_window: Option<WindowId>,
}

impl TabBarProcessor {
  pub(super) fn new(tab_bar_state: Arc<FairMutex<TabBarState<EventProxy>>>) -> TabBarProcessor {
    TabBarProcessor { 
      tab_bar_state,
      is_mouse_down: false,
      mouse_down_position: None,
      current_mouse_position: None,
      mouse_down_window: None,
      current_window: None,
    }
  }

  pub(super) fn handle_event(&mut self, 
    config: &Config, 
    size_info: &SizeInfo, 
    event: GlutinEvent<Event>,
    command_queue: &mut MultiWindowCommandQueue,
  ) -> (bool, bool, Option<CursorIcon>) {
    let mut tab_state_updated = false;
    let mut command_queue_updated = false;
    let mut is_mouse_event = false;
    let mut is_mouse_up = false;
    let mut is_dragging = false;
    let mut cursor_icon = None;

    if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
        use glutin::event::WindowEvent::*;
        use glutin::event::{ElementState, MouseButton};

        match event {
          CursorMoved { position, .. } => {
            if self.is_mouse_down && self.mouse_down_position.is_none() {
              self.mouse_down_position = Some(position);
            }

            if self.is_mouse_down {
              is_dragging = self.handle_tab_drag(config, size_info, command_queue);
            }

            self.current_mouse_position = Some(position);
            self.current_window = Some(window_id);
            
            // TODO update the current window based on the position, cursorEntered and 
            // cursorLeft are no help here
            if !is_dragging {
              tab_state_updated = self.handle_hover(config, size_info, &mut cursor_icon);
            }
            
            is_mouse_event = true;
          }

          MouseInput { state, button, .. } => {
            self.is_mouse_down = {
              let new_mouse_down = 
                state == ElementState::Pressed && 
                button == MouseButton::Left;

              if new_mouse_down && !self.is_mouse_down {
                self.mouse_down_position = None;
                self.mouse_down_window = Some(window_id);
                command_queue_updated = self.handle_mouse_down(config, size_info, command_queue);
              }

              if self.is_mouse_down && state == ElementState::Released {
                self.handle_mouse_up();
                is_mouse_up = true;
              }
              
              new_mouse_down
            };

            // if state == ElementState::Released {
            //     println!(
            //       "Mouse data current_window: {:?}, mouse_down_window: {:?}, mouse_down: {:?}, current_mouse: {:?}",
            //       self.current_window, self.mouse_down_window, self.mouse_down_position, self.current_mouse_position
            //     );    
            // }
            
            tab_state_updated = self.handle_hover(config, size_info, &mut cursor_icon);
            is_mouse_event = true;
          }

          _ => {}
        }
    }

    let need_redraw = command_queue_updated || tab_state_updated || is_dragging || is_mouse_up;

    let skip_processor_run = if is_mouse_event && self.current_mouse_position.is_some() {
      let tab_count = self.tab_bar_state.lock().term_tab_collection.lock().tab_count();
      tab_count > 1 && self.current_mouse_position.unwrap().y < config.window.tab_bar_height as f64
    } else {
      false
    };

    (need_redraw, skip_processor_run, cursor_icon)
  }

  fn handle_mouse_down(
    &self,
    config: &Config,
    size_info: &SizeInfo,
    command_queue: &mut MultiWindowCommandQueue
  ) -> bool {
    if self.current_mouse_position.is_none() {
      return false; 
    }

    let mouse_pos = self.current_mouse_position.unwrap();

    if let Some(pressed_tab) = self.get_tab_from_position(config, size_info, mouse_pos) {
      if self.is_close_button(pressed_tab, size_info) {
        command_queue.push(MultiWindowCommand::CloseTab(pressed_tab));
      } else {
        command_queue.push(MultiWindowCommand::ActivateTab(pressed_tab));
      }

      true
    } else {
      false 
    }
  }

  fn handle_mouse_up(&self) {
    self.tab_bar_state.lock().dragged_tab = None;
  }

  fn handle_tab_drag(
    &self,
    config: &Config,
    size_info: &SizeInfo,
    command_queue: &mut MultiWindowCommandQueue
  ) -> bool {
    let mouse_down_position = self.mouse_down_position.unwrap();

    if let Some(dragged_tab) = self.get_tab_from_position(config, size_info, mouse_down_position) {
      let current_mouse_position = self.current_mouse_position.unwrap();

      self.tab_bar_state.lock().dragged_tab = Some(DraggedTab {
        tab_id: dragged_tab,
        x: current_mouse_position.x - mouse_down_position.x,
        y: current_mouse_position.y - mouse_down_position.y,
      });

      true
    } else {
      false
    }
  }
  
  fn handle_hover(&mut self, config: &Config, size_info: &SizeInfo, cursor_icon: &mut Option<CursorIcon>) -> bool {
    let mut did_update = false;
        
    let hovered_tab = if self.current_mouse_position.is_some() {
      self.get_tab_from_position(
        config, size_info, self.current_mouse_position.unwrap()
      )
    } else {
      None
    };

    if self.tab_bar_state.lock().hovered_tab != hovered_tab {
      self.tab_bar_state.lock().hovered_tab = hovered_tab;
      did_update = true;
    }

    // Handle close button
    if let (Some(hovered_tab), Some(mouse_pos)) = (hovered_tab, self.current_mouse_position)  {
      if self.is_close_button(hovered_tab, size_info) {
        *cursor_icon = Some(CursorIcon::Hand);
      } else {
        *cursor_icon = Some(CursorIcon::default());
      }
    }
    
    did_update
  }

  fn get_tab_from_position(&self, config: &Config, size_info: &SizeInfo, position: LogicalPosition) -> Option<usize> {
    let dpr = size_info.dpr as f32;    
    let tab_count = self.tab_bar_state.lock().term_tab_collection.lock().tab_count();

    // No tab bar if only one tab is there
    if tab_count <= 1 {
      return None;
    }

    let tab_height = config.window.tab_bar_height as f32 * dpr;
    let y = position.y as f32 * dpr;
    
    if y < tab_height {
      Some((position.x as f32 * tab_count as f32 * dpr / size_info.width).floor() as usize)
    } else {
      None
    }
  }

  fn is_close_button(&self, tab_id: usize, size_info: &SizeInfo) -> bool {
    if let Some(mouse_pos) = self.current_mouse_position {
      let dpr = size_info.dpr as f32;
      let tab_count = self.tab_bar_state.lock().term_tab_collection.lock().tab_count();
      let tab_x = size_info.width * tab_id as f32 / tab_count as f32;
      let x_relative_to_tab = mouse_pos.x as f32 * dpr - tab_x;

      x_relative_to_tab < (CLOSE_ICON_PADDING + CLOSE_ICON_WIDTH * 2.) * dpr && x_relative_to_tab >= 0.
    } else {
      false
    }
  }
}

pub struct TabBarRenderer {
  term_tab_collection: Arc<FairMutex<TermTabCollection<EventProxy>>>,
  tab_bar_state: Arc<FairMutex<TabBarState<EventProxy>>>,
}

impl TabBarRenderer {
  pub fn new(
    tab_bar_state: Arc<FairMutex<TabBarState<EventProxy>>>,
    term_tab_collection: Arc<FairMutex<TermTabCollection<EventProxy>>>,
  ) -> TabBarRenderer {
    TabBarRenderer { 
      tab_bar_state,
      term_tab_collection,
    }
  }

  pub fn tab_bar_visible(&self) -> bool {
    self.term_tab_collection.lock().tab_count() > 1
  }

  pub fn render(
    &self,
    renderer: &mut QuadRenderer, 
    config: &Config, 
    size_info: &SizeInfo,
    glyph_cache: &mut GlyphCache,
  ) {
    if !self.tab_bar_visible() {
      return;
    }

    let (active_tab, tab_count) = {
      let tab_collection = self.term_tab_collection.lock();
      
      let active_tab_id = if tab_collection.active_tab().is_some() { 
        Some(tab_collection.active_tab().unwrap().tab_id) 
      } else { 
        None 
      };

      (active_tab_id, tab_collection.tab_count())
    };

    let (hovered_tab, dragged_tab) = {
      let tab_bar_state = self.tab_bar_state.lock();
      (tab_bar_state.hovered_tab, tab_bar_state.dragged_tab)
    };

    let border_width = 0.5;
    let tab_color = Rgb { r: 190, g: 190, b: 190 };
    let border_color = Rgb { r: 100, g: 100, b: 100 };
    let tab_width = size_info.width as f32 / tab_count as f32;    
    let dpr = size_info.dpr as f32;    
    let tab_height = config.window.tab_bar_height as f32 * dpr;

    // Tabs rects
    let mut backgrounds = Vec::new();
    let mut masks = Vec::new();

    for i in 0..tab_count {
      let is_dragging = dragged_tab.is_some() && dragged_tab.unwrap().tab_id == i;
      let is_active = Some(i) == active_tab;
      let is_hovered = Some(i) == hovered_tab;

      self.render_tab_rects(
        &mut backgrounds,
        &mut masks,
        i, 
        is_active && !is_dragging, 
        is_hovered, 
        None,
        tab_width, 
        tab_height, 
        border_width, 
        tab_color, 
        border_color,
      );
    }

    renderer.draw_rects(&size_info, backgrounds);

    // Titles
    let mut f = config.font.clone();
    // TODO bring from tab-bar config
    f.size = font::Size::new(f.size.as_f32_pts() * 0.85);
    f.offset.x = 0;
    let metrics = GlyphCache::static_metrics(f, size_info.dpr).unwrap();
    
    for i in 0..tab_count {
      let is_dragging = dragged_tab.is_some() && dragged_tab.unwrap().tab_id == i;

      if !is_dragging {
        self.render_tab_text(
          renderer, 
          config, 
          glyph_cache, 
          &metrics, 
          i,
          Some(i) == hovered_tab,
          dragged_tab,
          tab_width,
          tab_height,
          size_info.dpr,
          &size_info,
        );
      }
    }

    // Render masks on top of everything
    renderer.draw_rects(&size_info, masks);

    // Render the dragged item
    if dragged_tab.is_some()  {
      let tab_id = dragged_tab.unwrap().tab_id;
      let mut dragging_backgrounds = Vec::new();
      let mut dragging_masks = Vec::new();

      self.render_tab_rects(
        &mut dragging_backgrounds,
        &mut dragging_masks,
        tab_id, 
        true, 
        true, 
        dragged_tab,
        tab_width, 
        tab_height, 
        border_width, 
        tab_color, 
        border_color,
      );
      
      renderer.draw_rects(&size_info, dragging_backgrounds);

      self.render_tab_text(
        renderer, 
        config, 
        glyph_cache, 
        &metrics, 
        tab_id,
        true,
        dragged_tab,
        tab_width,
        tab_height,
        size_info.dpr,
        &size_info,
      );
    }
  }

  fn render_tab_rects(&self,
    rects: &mut Vec<RenderRect>,
    masks: &mut Vec<RenderRect>,
    tab_id: usize,
    is_active: bool, 
    is_hovered: bool,
    dragged_tab: Option<DraggedTab>,
    tab_width: f32, 
    tab_height: f32,
    border_width: f32,
    tab_color: Rgb,
    border_color: Rgb,
  ) {
    let active_tab_brightness_factor = 1.2;
    let hovered_tab_brightness_factor = 0.9;   
    let tab_x = (tab_id as f32) * tab_width;

    let (tab_x, tab_y, alpha) = if dragged_tab.is_some() && dragged_tab.unwrap().tab_id == tab_id { 
      let dt = dragged_tab.unwrap();
      (tab_x + dt.x as f32, 0., 1.)
    } else { 
      let tab_x = (tab_id as f32) * tab_width;
      (tab_x, 0., 1.)
    };

    let brightness_factor = if is_active {
        active_tab_brightness_factor 
    } else if is_hovered { 
        hovered_tab_brightness_factor
    } else {
        1.0
    };

    // Border
    rects.push(RenderRect::new(
        tab_x,
        tab_y,
        tab_width,
        tab_height,
        border_color * brightness_factor,
        alpha,
    ));
    
    // Content
    rects.push(RenderRect::new(
        tab_x + border_width,
        tab_y,
        tab_width - 2.0 * border_width,
        tab_height - 2.0 * border_width,
        tab_color * brightness_factor,
        alpha,
    ));

    // Mask (rendered on top of the texts)
    if !is_active {
      masks.push(RenderRect::new(
          tab_x + border_width,
          tab_y + border_width,
          tab_width - 2. * border_width,
          tab_height - 2. * border_width,
          tab_color,
          0.4,
      ));
    }
  }

  fn render_tab_text(&self,
    renderer: &mut QuadRenderer,
    config: &Config,
    glyph_cache: &mut GlyphCache,
    metrics: &font::Metrics,
    tab_id: usize,
    is_hovered: bool,
    dragged_tab: Option<DraggedTab>,
    tab_width: f32,
    tab_height: f32, 
    dpr: f64,
    size_info: &SizeInfo,
  ) {
    let tab_x = (tab_id as f32) * tab_width;

    let (tab_x, tab_y, alpha) = if dragged_tab.is_some() && dragged_tab.unwrap().tab_id == tab_id { 
      let dt = dragged_tab.unwrap();
      (tab_x + dt.x as f32, 0. , 1.) 
    } else { 
      (tab_x, 0., 1.)
    };

    let tab_title = self.term_tab_collection.lock().tab(tab_id).title();

    let cell_width = metrics.average_advance.floor().max(1.) as f32;
    let cell_height = metrics.line_height.floor().max(1.) as f32;

    let text_width = tab_title.len() as f32 * cell_width;

    let delta = (3.* cell_width + text_width + 2. * (CLOSE_ICON_WIDTH + CLOSE_ICON_PADDING) * dpr as f32) - tab_width;
    let tab_title_ellipsis = if delta > 0. {
      let cut_point = tab_title.len() - (delta / cell_width).floor() as usize;
      String::from(&tab_title[..cut_point]) + "..."
    } else {
      tab_title
    };

    let text_width = tab_title_ellipsis.len() as f32 * cell_width;

    let padding_x = (tab_x + tab_width / 2. - text_width / 2.).max(0.);

    let mut sm = SizeInfo {
      padding_x, 
      padding_top: tab_y + tab_height / 2. - cell_height / 2.,
      padding_y: 0.,
      width: size_info.width + padding_x,
      height: size_info.height,
      dpr: size_info.dpr,
      cell_width,
      cell_height,
    };

    renderer.resize(&sm);

    renderer.with_api(&config, &sm, |mut api| {
      api.render_string(
          &tab_title_ellipsis,
          Line(0),
          glyph_cache,
          None,
      );
    });

    // Close Icon
    if is_hovered {
        sm.padding_x = tab_x + CLOSE_ICON_PADDING * dpr as f32;
        renderer.resize(&sm);
        renderer.with_api(&config, &sm, |mut api| {
            // TODO config for this 'x'
            api.render_string(
                "x",
                Line(0),
                glyph_cache,
                None,
            );
        });
    }
  }
}
