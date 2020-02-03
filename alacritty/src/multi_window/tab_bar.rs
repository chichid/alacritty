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
const CLOSE_ICON_WIDTH: f32 = 10.0;

struct DraggingInfo {
  pub tab_id: usize,
  pub ghost_tab_index: Option<usize>,
  pub is_detached: bool,
  pub initial_tab_state: TabState,
}

#[derive (Clone)]
pub struct TabState {
  title: String,
  x: f32,
  y: f32,
  width: f32,
  height: f32,
  active: bool,
  hovered: bool,
}

pub struct TabBarState {
  pub(self) tabs: Vec<TabState>,
  pub(self) dragged_tab: Option<TabState>,
  pub(self) active_tab_index: Option<usize>,
  pub(self) hovered_tab: Option<usize>,
  pub(self) dragging_info: Option<DraggingInfo>,
}

impl TabBarState {
  pub (super) fn new() -> TabBarState {
    TabBarState {
      active_tab_index: None,
      hovered_tab: None,
      dragging_info: None,
      dragged_tab: None,
      tabs: Vec::new(),
    }
  }

  fn tab_count(&self) -> usize {
    self.tabs.len()
  }

  fn tab_state(&self, index: usize) -> &TabState {
    &self.tabs[index]
  }

  fn tab_bar_height(&self, size_info: &SizeInfo, config: &Config) -> f64 {
    size_info.dpr * config.window.tab_bar_height as f64
  }

  pub (super) fn update(&mut self, size_info: &SizeInfo, config: &Config, tab_collection: &TermTabCollection<EventProxy>) {
    let tab_count = tab_collection.tab_count();

    let is_dragging_ghost_detached = if let Some(dragging_info) = &self.dragging_info {
      dragging_info.is_detached
    } else {
      false
    };

    self.active_tab_index = if let Some(active_tab) = tab_collection.active_tab() {
      Some(active_tab.tab_id)
    } else {
      None
    };
    
    let mut tabs = Vec::with_capacity(tab_count);
    
    let height = self.tab_bar_height(size_info, config) as f32;

    let width = if is_dragging_ghost_detached {
      size_info.width / (tab_count - 1) as f32
    } else {
      size_info.width / tab_count as f32
    };

    let mut current_tab_x = 0.;

    for i in 0..tab_count {
      let is_dragging = if let Some(dragging_info) = &self.dragging_info {
        dragging_info.tab_id == i
      } else {
        false
      };

      if is_dragging && is_dragging_ghost_detached {
        continue;
      }

      let active = if let Some(active_tab_index) = self.active_tab_index {
        !is_dragging && active_tab_index == i
      } else {
        false
      };

      let hovered = if let Some(hovered_tab_index) = self.hovered_tab {
        !is_dragging && hovered_tab_index == i
      } else {
        false
      };
  
      let title = if is_dragging { 
        String::default()
      } else {
        tab_collection.tab(i).title()
      };

      let x = if let Some(dragging_info) = &self.dragging_info {
        if let Some(ghost_tab_index) = dragging_info.ghost_tab_index {
          if i == dragging_info.tab_id {
            ghost_tab_index as f32 * width
          } else if i >= ghost_tab_index && i < dragging_info.tab_id {
            current_tab_x + width
          } else if i <= ghost_tab_index && i > dragging_info.tab_id {
            current_tab_x - width
          } else {
            current_tab_x
          }
        } else {
          current_tab_x
        }
      } else {
        current_tab_x
      };

      tabs.push(TabState {
        x,
        y: 0.,
        width,
        height,
        title,
        active,
        hovered,
      });

      current_tab_x += width;
    }

    self.tabs = tabs;
  }

  fn update_dragging_info(&mut self, config: &Config, size_info: &SizeInfo, tab_id: usize, deltax: f64, deltay: f64) {
    let tab_state = if let Some(dragging_info) = &self.dragging_info {
      dragging_info.initial_tab_state.clone()
    } else {
      self.tab_state(tab_id).clone()
    };
    
    let is_detached = deltay > self.tab_bar_height(size_info, config) * 1.5;

    let x = tab_state.x + (deltax as f32);

    let y = if is_detached {
      0. + deltay as f32
    } else {
      0.
    };

    let ghost_tab_index = if is_detached {
      None
    } else {
      let ghost_tab_index = tab_id as i32 + (0.5 + deltax as f32 / tab_state.width as f32).floor() as i32;
      Some(ghost_tab_index.max(0) as usize)
    };

    let dragged_tab_state = TabState {
      x: x.max(0.).min(size_info.width - tab_state.width),
      y: y.max(0.).min(size_info.height * size_info.dpr as f32),
      width: tab_state.width,
      height: tab_state.height,
      title: tab_state.title.clone(),
      active: true,
      hovered: true,
    };

    self.dragged_tab = Some(dragged_tab_state);

    self.dragging_info = Some(DraggingInfo {
      tab_id,
      ghost_tab_index,
      is_detached,
      initial_tab_state: tab_state,
    });
  }

  fn update_hovered_tab(&mut self, hovered_tab: Option<usize>) -> bool {
    let did_update = self.hovered_tab != hovered_tab;
    self.hovered_tab = hovered_tab;
    did_update 
  }

  fn clear_dragging_info(&mut self) -> bool {
    let did_change = self.dragging_info.is_some();

    self.dragging_info = None;
    self.dragged_tab = None;

    did_change
  }
}

pub(super) struct TabBarProcessor {
  tab_bar_state: Arc<FairMutex<TabBarState>>,
  is_mouse_down: bool,
  mouse_down_position: Option<LogicalPosition>,
  current_mouse_position: Option<LogicalPosition>,
  mouse_down_window: Option<WindowId>,
  current_window: Option<WindowId>,
}

impl TabBarProcessor {
  pub(super) fn new(tab_bar_state: Arc<FairMutex<TabBarState>>) -> TabBarProcessor {
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
    tab_collection: &TermTabCollection<EventProxy>,
    config: &Config, 
    size_info: &SizeInfo, 
    event: GlutinEvent<Event>,
    command_queue: &mut MultiWindowCommandQueue,
  ) -> (bool, bool, Option<CursorIcon>) {
    let mut tab_state_updated = false;
    let mut is_mouse_event = false;
    let mut is_mouse_up = false;
    let mut is_dragging = false;
    let mut cursor_icon = None;

    if let GlutinEvent::WindowEvent { event, window_id, .. } = event {
        use glutin::event::WindowEvent::*;
        use glutin::event::{ElementState, MouseButton};

        match event {
          RedrawRequested => {
            self.tab_bar_state.lock().update(size_info, config, tab_collection);
          }

          CursorMoved { position, .. } => {
            if self.is_mouse_down && self.mouse_down_position.is_none() {
              self.mouse_down_position = Some(position);
            }

            if self.is_mouse_down {
              is_dragging = self.handle_tab_drag(config, size_info);

              if is_dragging {
                tab_state_updated = true;
              }
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
                self.handle_mouse_down(
                  window_id,
                  &self.current_mouse_position.unwrap(), 
                  config, 
                  size_info,
                  command_queue
                );
              }

              if self.is_mouse_down && state == ElementState::Released {
                self.handle_mouse_up();
                is_mouse_up = true;
              }
              
              new_mouse_down
            };

            tab_state_updated = self.handle_hover(config, size_info, &mut cursor_icon);
            is_mouse_event = true;
          }

          _ => {}
        }
    }

    let need_redraw = tab_state_updated || is_dragging || is_mouse_up;

    let skip_processor_run = if is_mouse_event && self.current_mouse_position.is_some() {
      let tab_count = self.tab_bar_state.lock().tab_count();
      tab_count > 1 && self.current_mouse_position.unwrap().y < config.window.tab_bar_height as f64
    } else {
      false
    };

    (need_redraw, skip_processor_run, cursor_icon)
  }

  fn handle_mouse_down(
    &self,
    window_id: WindowId,
    mouse_position: &LogicalPosition,
    config: &Config,
    size_info: &SizeInfo,
    command_queue: &mut MultiWindowCommandQueue
  ) {
    if let Some(pressed_tab) = self.get_tab_from_mouse_position(config, size_info, mouse_position) { 
      if self.is_hover_close_button(size_info) {
        command_queue.push(MultiWindowCommand::CloseTab(window_id, pressed_tab));
      } else {
        command_queue.push(MultiWindowCommand::ActivateTab(window_id, pressed_tab));
      }
    }
  }

  fn handle_mouse_up(&self) -> bool {
    self.tab_bar_state.lock().clear_dragging_info()
  }

  fn handle_tab_drag(&self, config: &Config, size_info: &SizeInfo) -> bool {
    let mouse_down_position = self.mouse_down_position.unwrap();

    if let Some(tab_id) = self.get_tab_from_mouse_position(config, size_info, &mouse_down_position) {
      let current_mouse_position = self.current_mouse_position.unwrap();
      let deltax = (current_mouse_position.x - mouse_down_position.x) * size_info.dpr;
      let deltay = (current_mouse_position.y - mouse_down_position.y) * size_info.dpr;
      
      self.tab_bar_state.lock().update_dragging_info(config, size_info, tab_id, deltax, deltay);

      true
    } else {
      self.tab_bar_state.lock().clear_dragging_info()
    }
  }
  
  fn handle_hover(&mut self, config: &Config, size_info: &SizeInfo, cursor_icon: &mut Option<CursorIcon>) -> bool {
    // Tab hover
    let hovered_tab = if let Some(current_mouse_position) = self.current_mouse_position {
      self.get_tab_from_mouse_position(config, size_info, &current_mouse_position)
    } else {
      None
    };

    let did_update = self.tab_bar_state.lock().update_hovered_tab(hovered_tab);

    // Handle Close button cursor
    if self.is_hover_close_button(size_info) {
      *cursor_icon = Some(CursorIcon::Hand);
    } else {
      *cursor_icon = Some(CursorIcon::default());
    }

    did_update
  }

  fn get_tab_from_mouse_position(&self, config: &Config, size_info: &SizeInfo, position: &LogicalPosition) -> Option<usize> {
    let dpr = size_info.dpr as f32;

    let is_dragging_detached = if let Some(dragging_info) = &self.tab_bar_state.lock().dragging_info {
      dragging_info.is_detached
    } else {
      false
    };

    let tab_count = if is_dragging_detached {
      self.tab_bar_state.lock().tab_count() + 1
    } else {
      self.tab_bar_state.lock().tab_count()
    };

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

  fn is_hover_close_button(&self, size_info: &SizeInfo) -> bool {
    let hovered_tab = if let Some(hovered_tab) = self.tab_bar_state.lock().hovered_tab {
      hovered_tab
    } else {
      return false;
    };

    if let Some(mouse_pos) = self.current_mouse_position {
      let hovered_tab_width = self.tab_bar_state.lock().tab_state(hovered_tab).width;
      let tab_x = (size_info.dpr as f32 * mouse_pos.x as f32) % hovered_tab_width as f32;

      tab_x < (CLOSE_ICON_PADDING + CLOSE_ICON_WIDTH * 2.)
    } else {
      false
    }
  }
}

pub struct TabBarRenderer {
  tab_bar_state: Arc<FairMutex<TabBarState>>,
}

impl TabBarRenderer {
  pub fn new(
    tab_bar_state: Arc<FairMutex<TabBarState>>,
  ) -> TabBarRenderer {
    TabBarRenderer { 
      tab_bar_state,
    }
  }

  pub fn tab_bar_visible(&self) -> bool {
    self.tab_bar_state.lock().tab_count() > 1
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

    let tab_bar_state = self.tab_bar_state.lock();
    let mut backgrounds = Vec::new();
    let mut masks = Vec::new();

    for i in 0..tab_bar_state.tab_count() {
      self.render_background(&tab_bar_state.tab_state(i), &mut backgrounds);
    }

    renderer.draw_rects(&size_info, backgrounds);
    
    let mut f = config.font.clone();
    f.size = font::Size::new(f.size.as_f32_pts() * 0.85);
    let metrics = GlyphCache::static_metrics(f, size_info.dpr).unwrap();
    
    for i in 0..tab_bar_state.tab_count() {
      self.render_title(&tab_bar_state.tab_state(i), &mut masks, &size_info, renderer, config, &metrics, glyph_cache);
    }

    renderer.draw_rects(&size_info, masks);
    
    if let Some(dragged_tab) = &tab_bar_state.dragged_tab {
      let mut dragged_tab_background = Vec::new();
      let mut dragged_tab_masks = Vec::new();
      self.render_background(dragged_tab, &mut dragged_tab_background);

      renderer.draw_rects(&size_info, dragged_tab_background);
      self.render_title(dragged_tab, &mut dragged_tab_masks, &size_info, renderer, config, &metrics, glyph_cache);
      renderer.draw_rects(&size_info, dragged_tab_masks);
    }
  }

  fn render_background(&self, tab_state: &TabState, backgrounds: &mut Vec<RenderRect>) {
    let TabState { x, y, width, height, active, hovered, .. } = *tab_state; 

    // Border
    let border = RenderRect::new(
      x,
      y,
      width,
      height,
      self.tab_color(active, hovered) * 0.7,
      1.,
    );

    backgrounds.push(border);
    
    // Content
    let border_width = self.border_width();

    let fill = RenderRect::new(
      x + border_width,
      y,
      width - 2.0 * border_width - 1.,
      height - 2.0 * border_width,
      self.tab_color(active, hovered),
      1.,
    );

    backgrounds.push(fill);
  }

  fn border_width(&self) -> f32 {
    // TODO get from config or theme
    0.7
  }

  fn render_title(&self, 
    tab_state: &TabState,
    masks: &mut Vec<RenderRect>, 
    size_info: &SizeInfo,
    renderer: &mut QuadRenderer, 
    config: &Config, 
    metrics: &font::Metrics,
    glyph_cache: &mut GlyphCache
  ) {
    let TabState { x, y, width, height, active, hovered, title, .. } = tab_state;
    let border_width = self.border_width();
    
    let cell_width = metrics.average_advance.floor().max(1.) as f32;
    let cell_height = metrics.line_height.floor().max(1.) as f32;

    let ellipsis_tab_title = self.ellipsis_tab_title(&title, cell_width, size_info.dpr as f32, *width);
    let text_width = ellipsis_tab_title.len() as f32 * cell_width;
    let padding_x = (x + width / 2. - text_width / 2.).max(0.);

    let mut sm = SizeInfo {
      padding_x, 
      padding_top: y + height / 2. - cell_height / 2.,
      padding_y: 0.,
      width: size_info.width + padding_x,
      height: size_info.height,
      dpr: size_info.dpr,
      cell_width,
      cell_height,
    };
    
    renderer.resize(&sm);

    renderer.with_api(config, &sm, |mut api| {
      api.render_string(
          &ellipsis_tab_title,
          Line(0),
          glyph_cache,
          None,
      );
    });

    // Close Icon
    if *hovered {
        sm.padding_x = x + CLOSE_ICON_PADDING * size_info.dpr as f32;
        renderer.resize(&sm);
        renderer.with_api(config, &sm, |mut api| {
            // TODO config for this 'x'
            api.render_string(
                "x",
                Line(0),
                glyph_cache,
                None,
            );
        });
    }

    // Mask (rendered on top of the texts)
    if !active {
      masks.push(RenderRect::new(
          x + border_width,
          y + border_width,
          width - 2. * border_width,
          height - 2. * border_width,
          self.tab_color(*active, *hovered),
          0.4,
      ));
    }
  }

  fn tab_color(&self, active: bool, hovered: bool) -> Rgb {
    // TODO Move to constant or config
    let color = Rgb { r: 190, g: 190, b: 190 };
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

  fn ellipsis_tab_title(&self, title: &str, cell_width: f32, dpr: f32, drag_aware_width: f32) -> String {
    let ellipsis = "...";
    let text_width = title.len() as f32 * cell_width;
    let title_padding = 4. * (CLOSE_ICON_WIDTH + CLOSE_ICON_PADDING) * dpr;
    let truncated_text_width = ellipsis.len() as f32 * cell_width + text_width + title_padding;
    let delta =  truncated_text_width - drag_aware_width;

    if delta > 0. {
      let cut_point = title.len() - (delta / cell_width).floor() as usize;
      String::from(&title[..cut_point]) + "..."
    } else {
      title.to_string()
    }
  }  
}

