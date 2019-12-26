// Copyright 2016 Joe Wilm, The Alacritty Project Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The display subsystem including window management, font rasterization, and
//! GPU drawing.
use std::f64;
use std::fmt;
use std::time::Instant;

use glutin::dpi::{PhysicalPosition, PhysicalSize};
use glutin::event::ModifiersState;
use glutin::event_loop::EventLoopWindowTarget;
use glutin::window::CursorIcon;
use log::{debug, info};
use parking_lot::MutexGuard;

use font::{self, Rasterize};

use alacritty_terminal::config::{Font, StartupMode};
use alacritty_terminal::event::{Event, OnResize};
use alacritty_terminal::index::Line;
use alacritty_terminal::message_bar::MessageBuffer;
use alacritty_terminal::meter::Meter;
use alacritty_terminal::renderer::rects::{RenderLines, RenderRect};
use alacritty_terminal::renderer::{self, GlyphCache, QuadRenderer};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::color::Rgb;
use alacritty_terminal::term::{RenderableCell, SizeInfo, Term, TermMode};

use crate::config::Config;
use crate::event::{DisplayUpdate, Mouse};
use crate::url::{Url, Urls};
use crate::window::{self, Window};

#[derive(Debug)]
pub enum Error {
    /// Error with window management
    Window(window::Error),

    /// Error dealing with fonts
    Font(font::Error),

    /// Error in renderer
    Render(renderer::Error),

    /// Error during buffer swap
    ContextError(glutin::ContextError),
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn (std::error::Error)> {
        match *self {
            Error::Window(ref err) => Some(err),
            Error::Font(ref err) => Some(err),
            Error::Render(ref err) => Some(err),
            Error::ContextError(ref err) => Some(err),
        }
    }

    fn description(&self) -> &str {
        match *self {
            Error::Window(ref err) => err.description(),
            Error::Font(ref err) => err.description(),
            Error::Render(ref err) => err.description(),
            Error::ContextError(ref err) => err.description(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Window(ref err) => err.fmt(f),
            Error::Font(ref err) => err.fmt(f),
            Error::Render(ref err) => err.fmt(f),
            Error::ContextError(ref err) => err.fmt(f),
        }
    }
}

impl From<window::Error> for Error {
    fn from(val: window::Error) -> Error {
        Error::Window(val)
    }
}

impl From<font::Error> for Error {
    fn from(val: font::Error) -> Error {
        Error::Font(val)
    }
}

impl From<renderer::Error> for Error {
    fn from(val: renderer::Error) -> Error {
        Error::Render(val)
    }
}

impl From<glutin::ContextError> for Error {
    fn from(val: glutin::ContextError) -> Error {
        Error::ContextError(val)
    }
}

/// The display wraps a window, font rasterizer, and GPU renderer
pub struct Display {
    pub size_info: SizeInfo,
    pub window: Window,
    pub urls: Urls,

    /// Currently highlighted URL.
    pub highlighted_url: Option<Url>,

    renderer: QuadRenderer,
    glyph_cache: GlyphCache,
    meter: Meter,
}

impl Display {
    pub fn new(config: &Config, estimated_dpr: f64, event_loop: &EventLoopWindowTarget<Event>) -> Result<Display, Error> {
        // Guess the target window dimensions
        let metrics = GlyphCache::static_metrics(config.font.clone(), estimated_dpr)?;
        let (cell_width, cell_height) = compute_cell_size(config, &metrics);
        let dimensions =
            GlyphCache::calculate_dimensions(config, estimated_dpr, cell_width, cell_height);

        debug!("Estimated DPR: {}", estimated_dpr);
        debug!("Estimated Cell Size: {} x {}", cell_width, cell_height);
        debug!("Estimated Dimensions: {:?}", dimensions);

        // Create the window where Alacritty will be displayed
        let logical = dimensions.map(|d| PhysicalSize::new(d.0, d.1).to_logical(estimated_dpr));

        // Spawn window
        let mut window = Window::new(event_loop, &config, logical)?;

        let dpr = window.hidpi_factor();
        info!("Device pixel ratio: {}", dpr);

        // get window properties for initializing the other subsystems
        let mut viewport_size = window.inner_size().to_physical(dpr);

        // Create renderer
        let mut renderer = QuadRenderer::new()?;

        let (glyph_cache, cell_width, cell_height) =
            Self::new_glyph_cache(dpr, &mut renderer, config)?;

        let mut padding_x = f32::from(config.window.padding.x) * dpr as f32;
        let mut padding_y = f32::from(config.window.padding.y) * dpr as f32;

        if let Some((width, height)) =
            GlyphCache::calculate_dimensions(config, dpr, cell_width, cell_height)
        {
            let PhysicalSize { width: w, height: h } = window.inner_size().to_physical(dpr);
            if (w - width).abs() < f64::EPSILON && (h - height).abs() < f64::EPSILON {
                info!("Estimated DPR correctly, skipping resize");
            } else {
                viewport_size = PhysicalSize::new(width, height);
                window.set_inner_size(viewport_size.to_logical(dpr));
            }
        } else if config.window.dynamic_padding {
            // Make sure additional padding is spread evenly
            padding_x = dynamic_padding(padding_x, viewport_size.width as f32, cell_width);
            padding_y = dynamic_padding(padding_y, viewport_size.height as f32, cell_height);
        }

        padding_x = padding_x.floor();
        padding_y = padding_y.floor();

        info!("Cell Size: {} x {}", cell_width, cell_height);
        info!("Padding: {} x {}", padding_x, padding_y);

        // Create new size with at least one column and row
        let size_info = SizeInfo {
            dpr,
            width: (viewport_size.width as f32).max(cell_width + 2. * padding_x),
            height: (viewport_size.height as f32).max(cell_height + 2. * padding_y),
            cell_width,
            cell_height,
            padding_x,
            padding_y,
        };

        // Update OpenGL projection
        renderer.resize(&size_info);

        // Clear screen
        let background_color = config.colors.primary.background;
        renderer.with_api(&config, &size_info, |api| {
            api.clear(background_color);
        });

        // We should call `clear` when window is offscreen, so when `window.show()` happens it
        // would be with background color instead of uninitialized surface.
        window.swap_buffers();

        window.set_visible(true);

        // Set window position
        //
        // TODO: replace `set_position` with `with_position` once available
        // Upstream issue: https://github.com/tomaka/winit/issues/806
        if let Some(position) = config.window.position {
            let physical = PhysicalPosition::from((position.x, position.y));
            let logical = physical.to_logical(dpr);
            window.set_outer_position(logical);
        }

        #[allow(clippy::single_match)]
        match config.window.startup_mode() {
            StartupMode::Fullscreen => window.set_fullscreen(true),
            #[cfg(target_os = "macos")]
            StartupMode::SimpleFullscreen => window.set_simple_fullscreen(true),
            #[cfg(not(any(target_os = "macos", windows)))]
            StartupMode::Maximized => window.set_maximized(true),
            _ => (),
        }

        Ok(Display {
            window,
            renderer,
            glyph_cache,
            meter: Meter::new(),
            size_info,
            urls: Urls::new(),
            highlighted_url: None,
        })
    }

    fn new_glyph_cache(
        dpr: f64,
        renderer: &mut QuadRenderer,
        config: &Config,
    ) -> Result<(GlyphCache, f32, f32), Error> {
        let font = config.font.clone();
        let rasterizer = font::Rasterizer::new(dpr as f32, config.font.use_thin_strokes())?;

        // Initialize glyph cache
        let glyph_cache = {
            info!("Initializing glyph cache...");
            let init_start = Instant::now();

            let cache =
                renderer.with_loader(|mut api| GlyphCache::new(rasterizer, &font, &mut api))?;

            let stop = init_start.elapsed();
            let stop_f = stop.as_secs() as f64 + f64::from(stop.subsec_nanos()) / 1_000_000_000f64;
            info!("... finished initializing glyph cache in {}s", stop_f);

            cache
        };

        // Need font metrics to resize the window properly. This suggests to me the
        // font metrics should be computed before creating the window in the first
        // place so that a resize is not needed.
        let (cw, ch) = compute_cell_size(config, &glyph_cache.font_metrics());

        Ok((glyph_cache, cw, ch))
    }

    /// Update font size and cell dimensions
    fn update_glyph_cache(&mut self, config: &Config, font: Font) {
        let size_info = &mut self.size_info;
        let cache = &mut self.glyph_cache;

        self.renderer.with_loader(|mut api| {
            let _ = cache.update_font_size(font, size_info.dpr, &mut api);
        });

        // Update cell size
        let (cell_width, cell_height) = compute_cell_size(config, &self.glyph_cache.font_metrics());
        size_info.cell_width = cell_width;
        size_info.cell_height = cell_height;
    }

    pub fn make_current(&mut self) {
        self.window.make_current();
        self.renderer.resize(&self.size_info);
    }

    pub fn request_resize(&mut self) {
        // Sync Size of the terminal and display
        let inner_size = self.window.inner_size();
        self.window.set_inner_size(glutin::dpi::LogicalSize::new(inner_size.width - 1.0, inner_size.height));
        self.window.set_inner_size(inner_size);
    }

    /// Process update events
    pub fn handle_update<T>(
        &mut self,
        terminal: &mut Term<T>,
        pty_resize_handle: &mut dyn OnResize,
        message_buffer: &MessageBuffer,
        config: &Config,
        update_pending: DisplayUpdate,
    ) {
        // Update font size and cell dimensions
        if let Some(font) = update_pending.font {
            self.update_glyph_cache(config, font);
        }

        let cell_width = self.size_info.cell_width;
        let cell_height = self.size_info.cell_height;

        // Recalculate padding
        let mut padding_x = f32::from(config.window.padding.x) * self.size_info.dpr as f32;
        let mut padding_y = f32::from(config.window.padding.y) * self.size_info.dpr as f32;

        // Update the window dimensions
        if let Some(size) = update_pending.dimensions {
            // Ensure we have at least one column and row
            self.size_info.width = (size.width as f32).max(cell_width + 2. * padding_x);
            self.size_info.height = (size.height as f32).max(cell_height + 2. * padding_y);
        }

        // Distribute excess padding equally on all sides
        if config.window.dynamic_padding {
            padding_x = dynamic_padding(padding_x, self.size_info.width, cell_width);
            padding_y = dynamic_padding(padding_y, self.size_info.height, cell_height);
        }

        self.size_info.padding_x = padding_x.floor() as f32;
        self.size_info.padding_y = padding_y.floor() as f32;

        let mut pty_size = self.size_info;

        // Subtract message bar lines from pty size
        if let Some(message) = message_buffer.message() {
            let lines = message.text(&self.size_info).len();
            pty_size.height -= pty_size.cell_height * lines as f32;
        }

        // Resize PTY
        pty_resize_handle.on_resize(&pty_size);

        // Resize terminal
        terminal.resize(&pty_size);

        // Resize renderer
        let physical =
            PhysicalSize::new(f64::from(self.size_info.width), f64::from(self.size_info.height));
        self.renderer.resize(&self.size_info);
        self.window.resize(physical);
    }

    /// Draw the screen
    ///
    /// A reference to Term whose state is being drawn must be provided.
    ///
    /// This call may block if vsync is enabled
    pub fn draw<T>(
        &mut self,
        terminal: MutexGuard<'_, Term<T>>,
        message_buffer: &MessageBuffer,
        config: &Config,
        mouse: &Mouse,
        mods: ModifiersState,
    ) {
        let grid_cells: Vec<RenderableCell> = terminal.renderable_cells(config).collect();
        let visual_bell_intensity = terminal.visual_bell.intensity();
        let background_color = terminal.background_color();
        let metrics = self.glyph_cache.font_metrics();
        let glyph_cache = &mut self.glyph_cache;
        let size_info = self.size_info;

        let selection = !terminal.selection().as_ref().map(Selection::is_empty).unwrap_or(true);
        let mouse_mode = terminal.mode().intersects(TermMode::MOUSE_MODE);

        // Update IME position
        #[cfg(not(windows))]
        self.window.update_ime_position(&terminal, &self.size_info);

        // Drop terminal as early as possible to free lock
        drop(terminal);

        self.renderer.with_api(&config, &size_info, |api| {
            api.clear(background_color);
        });

        let mut lines = RenderLines::new();
        let mut urls = Urls::new();

        // Draw grid
        {
            let _sampler = self.meter.sampler();

            self.renderer.with_api(&config, &size_info, |mut api| {
                // Iterate over all non-empty cells in the grid
                for cell in grid_cells {
                    // Update URL underlines
                    urls.update(size_info.cols().0, cell);

                    // Update underline/strikeout
                    lines.update(cell);

                    // Draw the cell
                    api.render_cell(cell, glyph_cache);
                }
            });
        }

        let mut rects = lines.rects(&metrics, &size_info);

        // Update visible URLs
        self.urls = urls;
        if let Some(url) = self.urls.highlighted(config, mouse, mods, mouse_mode, selection) {
            rects.append(&mut url.rects(&metrics, &size_info));

            self.window.set_mouse_cursor(CursorIcon::Hand);

            self.highlighted_url = Some(url);
        } else if self.highlighted_url.is_some() {
            self.highlighted_url = None;

            if mouse_mode {
                self.window.set_mouse_cursor(CursorIcon::Default);
            } else {
                self.window.set_mouse_cursor(CursorIcon::Text);
            }
        }

        // Push visual bell after url/underline/strikeout rects
        if visual_bell_intensity != 0. {
            let visual_bell_rect = RenderRect::new(
                0.,
                0.,
                size_info.width,
                size_info.height,
                config.visual_bell.color,
                visual_bell_intensity as f32,
            );
            rects.push(visual_bell_rect);
        }

        if let Some(message) = message_buffer.message() {
            let text = message.text(&size_info);

            // Create a new rectangle for the background
            let start_line = size_info.lines().0 - text.len();
            let y = size_info.cell_height.mul_add(start_line as f32, size_info.padding_y);
            let message_bar_rect =
                RenderRect::new(0., y, size_info.width, size_info.height - y, message.color(), 1.);

            // Push message_bar in the end, so it'll be above all other content
            rects.push(message_bar_rect);

            // Draw rectangles
            self.renderer.draw_rects(&size_info, rects);

            // Relay messages to the user
            let mut offset = 1;
            for message_text in text.iter().rev() {
                self.renderer.with_api(&config, &size_info, |mut api| {
                    api.render_string(
                        &message_text,
                        Line(size_info.lines().saturating_sub(offset)),
                        glyph_cache,
                        None,
                    );
                });
                offset += 1;
            }
        } else {
            // Draw rectangles
            self.renderer.draw_rects(&size_info, rects);
        }

        render_tabs(&mut self.renderer, &config, &size_info, glyph_cache);

        // Draw render timer
        if config.render_timer() {
            let timing = format!("{:.3} usec", self.meter.average());
            let color = Rgb { r: 0xd5, g: 0x4e, b: 0x53 };
            self.renderer.with_api(&config, &size_info, |mut api| {
                api.render_string(&timing[..], size_info.lines() - 2, glyph_cache, Some(color));
            });
        }

        self.window.swap_buffers();
    }   
}

fn render_tabs(renderer: &mut QuadRenderer, config: &Config, size_info: &SizeInfo, glyph_cache: &mut GlyphCache) {
    let active_tab = 2;
    let hovered_tab = 1;
    let tab_count = 4;
    let dpr = size_info.dpr as f32;

    let tab_font_size = 11.;
    let tab_width = size_info.width as f32 / tab_count as f32;
    let tab_height = 26. * dpr;
    let tab_color = Rgb { r: 190, g: 190, b: 190 };

    let border_color = Rgb { r: 150, g: 150, b: 150 };
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
    renderer.with_loader(|mut api| {
        let mut f = config.font.clone();
        f.size = font::Size::new(tab_font_size);
        f.offset.x = 0;   
        let _ = glyph_cache.update_font_size(f, size_info.dpr, &mut api);
    });

    let mut rects = Vec::new();

    for i in 0..tab_count {
        let tab_title = format!("~/Github/fish - Tab {}", i);
        let text_width = tab_title.len() as f32 * tab_font_size;
        let text_height = tab_font_size * size_info.dpr as f32;
        let mut sm = *size_info;

        sm.padding_x = (i as f32) * tab_width + tab_width / 2. - text_width / 2.;
        sm.padding_y = tab_height / 4. - text_height / 4.;
        sm.width = size_info.width + sm.padding_x;
        sm.cell_width = tab_font_size as f32 + 1.;
        
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
            sm.padding_x = (i as f32) * tab_width + close_icon_padding;
            renderer.resize(&sm);
            renderer.with_api(&config, &sm, |mut api| {
                api.render_string(
                    "⨉",
                    Line(0),
                    glyph_cache,
                    None,
                );
            });
        }

        // Content
        if i != active_tab {
            let tab_x = (i as f32) * tab_width;

            rects.push(RenderRect::new(
                tab_x,
                0.,
                tab_width,
                tab_height,
                tab_color,
                0.35,
            ));
        }
    }

    renderer.draw_rects(&size_info, rects);

    renderer.with_loader(|mut api| {
        let _ = glyph_cache.update_font_size(config.font.clone(), size_info.dpr, &mut api);
    });
}

/// Calculate padding to spread it evenly around the terminal content
#[inline]
fn dynamic_padding(padding: f32, dimension: f32, cell_dimension: f32) -> f32 {
    padding + ((dimension - 2. * padding) % cell_dimension) / 2.
}

/// Calculate the cell dimensions based on font metrics.
#[inline]
fn compute_cell_size(config: &Config, metrics: &font::Metrics) -> (f32, f32) {
    let offset_x = f64::from(config.font.offset.x);
    let offset_y = f64::from(config.font.offset.y);
    (
        ((metrics.average_advance + offset_x) as f32).floor().max(1.),
        ((metrics.line_height + offset_y) as f32).floor().max(1.),
    )
}
