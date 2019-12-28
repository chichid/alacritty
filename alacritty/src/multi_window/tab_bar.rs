use crate::event::EventProxy;
use std::sync::Arc;

use glutin::event::Event as GlutinEvent;
use glutin::window::WindowId;

use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::index::Line;
use alacritty_terminal::term::color::Rgb;
use alacritty_terminal::renderer::GlyphCache;
use alacritty_terminal::term::SizeInfo;
use alacritty_terminal::renderer::QuadRenderer;
use alacritty_terminal::renderer::rects::{RenderLines, RenderRect};

use crate::config::Config;
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
      (tab_collection.get_active_tab(), tab_collection.tab_count())
    };

    if active_tab.is_none() {
      return;
    }

    let active_tab = active_tab.unwrap().tab_id;
    let hovered_tab = active_tab;
    let dpr = size_info.dpr as f32;    
    let tab_font_size_factor = 0.75;
    let tab_width = size_info.width as f32 / tab_count as f32;
    let tab_height = config.window.tab_bar_height as f32 * dpr;
    let tab_color = Rgb { r: 190, g: 190, b: 190 };
    let border_color = Rgb { r: 100, g: 100, b: 100 };
    let border_width = 0.7;
    let active_tab_brightness_factor = 1.1;
    let hovered_tab_brightness_factor = 0.9;   
    let close_icon_padding = 10.0 * dpr;

    // Tabs background
    let mut rects = Vec::new();

    for i in 0..tab_count {
        let tab_x = (i as f32) * tab_width;

        let brightness_factor = if i == active_tab {
            active_tab_brightness_factor 
        } else if i == hovered_tab { 
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
    let offset_x = 1;
    f.offset.x = offset_x;
    f.offset.y = 10;

    let metrics = GlyphCache::static_metrics(f, size_info.dpr).unwrap();
    let mut average_advance = metrics.average_advance;
    let mut line_height = metrics.line_height;
    let mut rects = Vec::new();

    for i in 0..tab_count {
        let tab_x = (i as f32) * tab_width;
        let tab_title = format!("~/Github/fish - Tab {}", i);
        let cell_width = offset_x as f32 + average_advance.floor().max(1.) as f32;

        let text_width = tab_title.len() as f32 * cell_width;
        let text_height = line_height.floor().max(1.) as f32;

        let mut sm = *size_info;
        sm.padding_x = ((i as f32) * tab_width + tab_width / 2. - text_width / 2.).max(0.);
        sm.padding_top = 0.0;
        sm.width = size_info.width + sm.padding_x;
        sm.cell_width = cell_width;
        
        println!("SM is {:?} text_width {:?}", sm, text_width);
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
        if i == hovered_tab {
            sm.padding_x = tab_x + close_icon_padding;
            renderer.resize(&sm);
            renderer.with_api(&config, &sm, |mut api| {
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
