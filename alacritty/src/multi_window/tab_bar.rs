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
pub struct DraggingInfo {
  pub tab_id: usize,
  pub x: f64,
  pub y: f64,
}

pub struct TabBarState<T> {
  pub hovered_tab: Option<usize>,
  pub dragged_tab: Option<DraggingInfo>,
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

      self.tab_bar_state.lock().dragged_tab = Some(DraggingInfo {
        tab_id: dragged_tab,
        x: (current_mouse_position.x - mouse_down_position.x) * size_info.dpr,
        y: (current_mouse_position.y - mouse_down_position.y) * size_info.dpr,
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
    
    // Titles
    // TODO bring from tab-bar config
    let mut f = config.font.clone();
    f.size = font::Size::new(f.size.as_f32_pts() * 0.85);
    let metrics = GlyphCache::static_metrics(f, size_info.dpr).unwrap();
    
    // Create a tab renderer
    let mut tab_renderer = TabRenderer {
      backgrounds: Vec::new(),
      masks: Vec::new(),
      config,
      glyph_cache,
      border_width,
      size_info,
      width: tab_width,
      height: tab_height,
      metrics,
      border_color,
      tab_color,
      active_tab,
      hovered_tab,
      dragging_info: dragged_tab,
    };

    // Draw backgrounds
    for i in 0..tab_count {
      tab_renderer.render_background(i);
    }

    renderer.draw_rects(&size_info, tab_renderer.backgrounds.clone());
    
    // Draw Titles
    for i in 0..tab_count {
      let title = self.term_tab_collection.lock().tab(i).title();
      tab_renderer.render_title(i, title, renderer);
    }

    renderer.draw_rects(&size_info, tab_renderer.masks.clone());
    
    // Draw dragged tab text
    if let Some(dragged_tab) = dragged_tab {
      let tab_index = dragged_tab.tab_id;
      let dragged_tab_title = self.term_tab_collection.lock().tab(tab_index).title();
      tab_renderer.render_title(tab_index, dragged_tab_title, renderer);
    }
  }
}

struct TabRenderer<'a> {
  config: &'a Config,
  glyph_cache: &'a mut GlyphCache,
  backgrounds: Vec<RenderRect>,
  masks: Vec<RenderRect>,
  border_width: f32,
  size_info: &'a SizeInfo,
  width: f32,
  height: f32,
  metrics: font::Metrics,
  border_color: Rgb,
  tab_color: Rgb,
  active_tab: Option<usize>,
  hovered_tab: Option<usize>,
  dragging_info: Option<DraggingInfo>,
}

impl<'a> TabRenderer<'a> {
  fn render_background(&mut self, tab_index: usize) {
    let x = self.tab_x(tab_index); 
    
    let is_dragging = self.dragging_info.is_some() && self.dragging_info.unwrap().tab_id == tab_index;
    let active = self.active_tab == Some(tab_index);
    let hovered = self.hovered_tab == Some(tab_index);

    // Border
    let border = RenderRect::new(
      x,
      0.,
      self.width,
      self.height,
      TabRenderer::tab_state_color(self.border_color, active, hovered),
      1.,
    );

    self.backgrounds.push(border);
    
    // Content
    let fill = RenderRect::new(
      x + self.border_width,
      0.,
      self.width - 2.0 * self.border_width,
      self.height - 2.0 * self.border_width,
      TabRenderer::tab_state_color(self.tab_color, active, hovered),
      1.,
    );

    self.backgrounds.push(fill);

    // Dragging placeholder
    if is_dragging {
      let mut drag_placeholder_fill = fill;
      drag_placeholder_fill.x = tab_index as f32 * self.width;
      drag_placeholder_fill.color = TabRenderer::tab_state_color(self.tab_color, false, false);
      self.backgrounds.push(drag_placeholder_fill);

      self.masks.push(border);
      self.masks.push(fill);
    }
  }

  fn tab_state_color(color: Rgb, active: bool, hovered: bool) -> Rgb {
    // TODO Move to constant or config
    let active_tab_brightness_factor = 1.2;
    let hovered_tab_brightness_factor = 0.9;   

    let factor = if active {
      active_tab_brightness_factor 
    } else if hovered { 
        hovered_tab_brightness_factor
    } else {
        1.0
    };

    color * factor 
  }

  fn render_title(&mut self, tab_index:usize, title: String, renderer: &mut QuadRenderer) {
    let tab_x = self.tab_x(tab_index);
    let tab_y = 0.;

    let active = self.active_tab == Some(tab_index);
    let hovered = self.hovered_tab == Some(tab_index);

    let (cell_width, cell_height) = self.cell_dimensions();
    let ellipsis_tab_title = self.ellipsis_tab_title(title);
    let text_width = ellipsis_tab_title.len() as f32 * cell_width;
    let padding_x = (tab_x + self.width / 2. - text_width / 2.).max(0.);

    let mut sm = SizeInfo {
      padding_x, 
      padding_top: tab_y + self.height / 2. - cell_height / 2.,
      padding_y: 0.,
      width: self.size_info.width + padding_x,
      height: self.size_info.height,
      dpr: self.size_info.dpr,
      cell_width,
      cell_height,
    };
    
    renderer.resize(&sm);

    renderer.with_api(self.config, &sm, |mut api| {
      api.render_string(
          &ellipsis_tab_title,
          Line(0),
          self.glyph_cache,
          None,
      );
    });

    // Close Icon
    if hovered {
        sm.padding_x = tab_x + CLOSE_ICON_PADDING * self.size_info.dpr as f32;
        renderer.resize(&sm);
        renderer.with_api(&self.config, &sm, |mut api| {
            // TODO config for this 'x'
            api.render_string(
                "x",
                Line(0),
                self.glyph_cache,
                None,
            );
        });
    }

    // Mask (rendered on top of the texts)
    if !active {
      self.masks.push(RenderRect::new(
          tab_x + self.border_width,
          tab_y + self.border_width,
          self.width - 2. * self.border_width,
          self.height - 2. * self.border_width,
          self.tab_color,
          0.4,
      ));
    }
  }

  fn ellipsis_tab_title(&self, title: String) -> String {
    let ellipsis = "...";
    let (cell_width, _) = self.cell_dimensions();
    let text_width = title.len() as f32 * cell_width;
    let dpr = self.size_info.dpr as f32;
    let truncated_text_width = ellipsis.len() as f32 * cell_width + text_width + 2. * (CLOSE_ICON_WIDTH + CLOSE_ICON_PADDING) * dpr;
    let delta =  truncated_text_width - self.width;

    if delta > 0. {
      let cut_point = title.len() - (delta / cell_width).floor() as usize;
      String::from(&title[..cut_point]) + "..."
    } else {
      title.clone()
    }
  }

  fn cell_dimensions(&self) -> (f32, f32) {
    let cell_width = self.metrics.average_advance.floor().max(1.) as f32;
    let cell_height = self.metrics.line_height.floor().max(1.) as f32;
    
    (cell_width, cell_height)
  }

  fn tab_x(&self, tab_index: usize) -> f32 {
    let x = tab_index as f32 * self.width;

    if let Some(dragging_info) = self.dragging_info {
      if dragging_info.tab_id == tab_index {
        x + dragging_info.x as f32
      } else {
        x as f32
      }
    } else {
      x as f32
    }
  }
}

