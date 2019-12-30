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

pub struct DropZone {
  tab_id: Option<usize>,
  window_id: WindowId,
}

pub struct TabBarState<T> {
  pub current_dragging_tab: Option<usize>,
  pub hovered_tab: Option<usize>,
  pub current_drop_zone: Option<DropZone>,
  term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>,
}

impl<T> TabBarState<T> {
  pub(super) fn new(term_tab_collection: Arc<FairMutex<TermTabCollection<T>>>) -> TabBarState<T> {
    TabBarState { 
      current_dragging_tab: None,
      current_drop_zone: None,
      hovered_tab: None,
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
              self.handle_tab_drag(config, size_info, command_queue);
            }

            self.current_mouse_position = Some(position);
            self.current_window = Some(window_id);
            
            // TODO update the current window based on the position, cursorEntered and 
            // cursorLeft are no help here
            tab_state_updated = self.handle_hover(config, size_info, &mut cursor_icon);
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
                command_queue_updated = self.handle_tab_pressed(config, size_info, command_queue);
              }
              
              new_mouse_down
            };

            tab_state_updated = self.handle_hover(config, size_info, &mut cursor_icon);
            is_mouse_event = true;
          }

          _ => {}
        }
    }

    let need_redraw = command_queue_updated || tab_state_updated;

    let skip_processor_run = if is_mouse_event && self.current_mouse_position.is_some() {
      let tab_count = self.tab_bar_state.lock().term_tab_collection.lock().tab_count();
      tab_count > 1 && self.current_mouse_position.unwrap().y < config.window.tab_bar_height as f64
    } else {
      false
    };

    (need_redraw, skip_processor_run, cursor_icon)
  }

  fn handle_tab_pressed(
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

  fn handle_tab_drag(
    &self,
    config: &Config,
    size_info: &SizeInfo,
    command_queue: &mut MultiWindowCommandQueue
  ) {
    let drag_tab = if self.mouse_down_position.is_some() {
      self.get_tab_from_position(
        config, size_info, self.mouse_down_position.unwrap()
      )
    } else {
      None
    };

    if let Some(drag_tab) = drag_tab {
      println!("Handle dragging {:?}", drag_tab);
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

    // println!(
    //   "Mouse data current_window: {:?}, mouse_down_window: {:?}, mouse_down: {:?}, current_mouse: {:?}",
    //   self.current_window, self.mouse_down_window, self.mouse_down_position, self.current_mouse_position
    // );
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
      Some((position.x as f32 * tab_count as f32 / size_info.width).floor() as usize)
    } else {
      None
    }
  }

  fn is_close_button(&self, tab_id: usize, size_info: &SizeInfo) -> bool {
    if let Some(mouse_pos) = self.current_mouse_position {
      let tab_count = self.tab_bar_state.lock().term_tab_collection.lock().tab_count();
      let tab_x = size_info.width * tab_id as f32 / tab_count as f32;
      let x_relative_to_tab = mouse_pos.x as f32 - tab_x;

      x_relative_to_tab < CLOSE_ICON_PADDING + CLOSE_ICON_WIDTH
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
      (tab_collection.active_tab(), tab_collection.tab_count())
    };

    if active_tab.is_none() {
      return;
    }

    let active_tab = active_tab.unwrap().tab_id;
    let hovered_tab = self.tab_bar_state.lock().hovered_tab;
    let dpr = size_info.dpr as f32;    
    let tab_width = size_info.width as f32 / tab_count as f32;
    let tab_height = config.window.tab_bar_height as f32 * dpr;
    let tab_color = Rgb { r: 190, g: 190, b: 190 };
    let border_color = Rgb { r: 100, g: 100, b: 100 };
    let border_width = 0.5;
    let active_tab_brightness_factor = 1.2;
    let hovered_tab_brightness_factor = 0.9;   
    let close_icon_padding = CLOSE_ICON_PADDING * dpr;

    // Tabs background
    let mut rects = Vec::new();

    for i in 0..tab_count {
        let tab_x = (i as f32) * tab_width;

        let brightness_factor = if i == active_tab {
            active_tab_brightness_factor 
        } else if Some(i) == hovered_tab { 
            hovered_tab_brightness_factor
        } else {
            1.0
        };

        // Border
        rects.push(RenderRect::new(
            tab_x,
            0.,
            tab_width,
            tab_height,
            border_color * brightness_factor,
            1.,
        ));
       
        // Content
        rects.push(RenderRect::new(
            tab_x + border_width,
            0.,
            tab_width - 2.0 * border_width,
            tab_height - 2.0 * border_width,
            tab_color * brightness_factor,
            1.,
        ));
    }

    renderer.draw_rects(&size_info, rects);

    // Titles
    let mut f = config.font.clone();
    f.offset.x = 0;
    f.offset.y = 10;

    let metrics = GlyphCache::static_metrics(f, size_info.dpr).unwrap();
    let average_advance = metrics.average_advance;
    let line_height = metrics.line_height;
    let mut rects = Vec::new();

    for i in 0..tab_count {
        let tab_x = (i as f32) * tab_width;
        let tab_title = self.term_tab_collection.lock().tab(i).title();
        let cell_width = average_advance.floor().max(1.) as f32;

        let text_width = tab_title.len() as f32 * cell_width;
        let text_height = line_height.floor().max(1.) as f32;

        let mut sm = *size_info;
        sm.padding_x = ((i as f32) * tab_width + tab_width / 2. - text_width / 2.).max(0.);
        sm.padding_top = 0.0;
        sm.width = size_info.width + sm.padding_x;
        sm.cell_width = cell_width;
        
        renderer.resize(&sm);

        renderer.with_api(&config, &sm, |mut api| {
            api.render_string(
                &tab_title,
                Line(0),
                glyph_cache,
                None,
            );
        });

        // Close Icon
        if Some(i) == hovered_tab {
            sm.padding_x = tab_x + close_icon_padding;
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

        // Inactive tabs mask
        if i != active_tab {
            rects.push(RenderRect::new(
                tab_x + border_width,
                0. + border_width,
                tab_width - 2. * border_width,
                tab_height - 2. * border_width,
                tab_color,
                0.4,
            ));
        }
    }

    renderer.draw_rects(&size_info, rects);
}
}
