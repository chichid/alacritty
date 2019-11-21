//! Process window events
use std::sync::Arc;
use alacritty_terminal::sync::FairMutex;
use crate::display_context::DisplayContext;
use crate::display_context::DisplayContextMap;
use alacritty_terminal::event_loop::Notifier;
use std::borrow::Cow;
use std::cmp::max;
use std::env;
#[cfg(unix)]
use std::fs;
use std::fs::File;
use std::io::Write;
use std::time::Instant;

use glutin::dpi::PhysicalSize;
use glutin::event_loop::EventLoop as GlutinEventLoop;
use glutin::event::{ElementState, Event as GlutinEvent, ModifiersState, MouseButton};
use glutin::event_loop::{ControlFlow, EventLoop, EventLoopProxy};
use glutin::platform::desktop::EventLoopExtDesktop;
use log::{debug, info, warn};
use serde_json as json;

use font::Size;

use alacritty_terminal::clipboard::ClipboardType;
use alacritty_terminal::config::Font;
use alacritty_terminal::config::LOG_TARGET_CONFIG;
use alacritty_terminal::event::{Event, EventListener, Notify};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::message_bar::{Message, MessageBuffer};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{SizeInfo, Term};
#[cfg(not(windows))]
use alacritty_terminal::tty;
use alacritty_terminal::util::{limit, start_daemon};

use crate::config;
use crate::config::Config;
use crate::display::Display;
use crate::input::{self, FONT_SIZE_STEP};
use crate::window::Window;
use crate::term_tabs::TermTabCollection;
use crate::display_context::{DisplayCommandQueue, DisplayCommand};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct DisplayUpdate {
    pub dimensions: Option<PhysicalSize>,
    pub message_buffer: Option<()>,
    pub font: Option<Font>,
}

impl DisplayUpdate {
    fn is_empty(&self) -> bool {
        self.dimensions.is_none() && self.font.is_none() && self.message_buffer.is_none()
    }
}

pub struct ActionContext<'a, N, T> {
    pub notifier: &'a mut N,
    pub terminal: &'a mut Term<T>,
    pub display_command_queue: &'a mut DisplayCommandQueue,
    pub display_context_map: &'a mut DisplayContextMap,
    pub terminal_tab_collection: &'a mut TermTabCollection<T>,
    pub size_info: &'a mut SizeInfo,
    pub mouse: &'a mut Mouse,
    pub received_count: &'a mut usize,
    pub suppress_chars: &'a mut bool,
    pub modifiers: &'a mut ModifiersState,
    pub window: &'a mut Window,
    pub message_buffer: &'a mut MessageBuffer,
    pub display_update_pending: &'a mut DisplayUpdate,
    pub config: &'a mut Config,
    font_size: &'a mut Size,
}

impl<'a, N: Notify + 'a, T: 'static + EventListener + Clone + Send> input::ActionContext<T> for ActionContext<'a, N, T> {
    fn write_to_pty<B: Into<Cow<'static, [u8]>>>(&mut self, val: B) {
        self.notifier.notify(val);
    }

    fn size_info(&self) -> SizeInfo {
        *self.size_info
    }

    fn scroll(&mut self, scroll: Scroll) {
        self.terminal.scroll_display(scroll);

        if let ElementState::Pressed = self.mouse().left_button_state {
            let (x, y) = (self.mouse().x, self.mouse().y);
            let size_info = self.size_info();
            let point = size_info.pixels_to_coords(x, y);
            let cell_side = self.mouse().cell_side;
            self.update_selection(Point { line: point.line, col: point.col }, cell_side);
        }
    }

    fn copy_selection(&mut self, ty: ClipboardType) {
        if let Some(selected) = self.terminal.selection_to_string() {
            if !selected.is_empty() {
                self.terminal.clipboard().store(ty, selected);
            }
        }
    }

    fn selection_is_empty(&self) -> bool {
        self.terminal.selection().as_ref().map(Selection::is_empty).unwrap_or(true)
    }

    fn clear_selection(&mut self) {
        *self.terminal.selection_mut() = None;
        self.terminal.dirty = true;
    }

    fn update_selection(&mut self, point: Point, side: Side) {
        let point = self.terminal.visible_to_buffer(point);

        // Update selection if one exists
        if let Some(ref mut selection) = self.terminal.selection_mut() {
            selection.update(point, side);
        }

        self.terminal.dirty = true;
    }

    fn simple_selection(&mut self, point: Point, side: Side) {
        let point = self.terminal.visible_to_buffer(point);
        *self.terminal.selection_mut() = Some(Selection::simple(point, side));
        self.terminal.dirty = true;
    }

    fn block_selection(&mut self, point: Point, side: Side) {
        let point = self.terminal.visible_to_buffer(point);
        *self.terminal.selection_mut() = Some(Selection::block(point, side));
        self.terminal.dirty = true;
    }

    fn semantic_selection(&mut self, point: Point) {
        let point = self.terminal.visible_to_buffer(point);
        *self.terminal.selection_mut() = Some(Selection::semantic(point));
        self.terminal.dirty = true;
    }

    fn line_selection(&mut self, point: Point) {
        let point = self.terminal.visible_to_buffer(point);
        *self.terminal.selection_mut() = Some(Selection::lines(point));
        self.terminal.dirty = true;
    }

    fn mouse_coords(&self) -> Option<Point> {
        let x = self.mouse.x as usize;
        let y = self.mouse.y as usize;

        if self.size_info.contains_point(x, y) {
            Some(self.size_info.pixels_to_coords(x, y))
        } else {
            None
        }
    }

    #[inline]
    fn mouse_mut(&mut self) -> &mut Mouse {
        self.mouse
    }

    #[inline]
    fn mouse(&self) -> &Mouse {
        self.mouse
    }

    #[inline]
    fn received_count(&mut self) -> &mut usize {
        &mut self.received_count
    }

    #[inline]
    fn suppress_chars(&mut self) -> &mut bool {
        &mut self.suppress_chars
    }

    #[inline]
    fn modifiers(&mut self) -> &mut ModifiersState {
        &mut self.modifiers
    }

    #[inline]
    fn window(&self) -> &Window {
        self.window
    }

    #[inline]
    fn window_mut(&mut self) -> &mut Window {
        self.window
    }

    #[inline]
    fn terminal(&self) -> &Term<T> {
        self.terminal
    }

    #[inline]
    fn terminal_mut(&mut self) -> &mut Term<T> {
        self.terminal
    }

    fn spawn_new_instance(&mut self) {
        // self.display_context_map.push_display_context();
        self.display_command_queue.push(DisplayCommand::CreateDisplay);
    }

    fn spawn_new_tab(&mut self) {
        self.terminal_tab_collection.push_tab();
    }

    fn activate_tab(&mut self, tab_id: usize) {
        self.terminal_tab_collection.activate_tab(tab_id);
    }

    fn close_current_tab(&mut self) {
        self.terminal_tab_collection.close_current_tab();
    }

    fn close_tab(&mut self, tab_id: usize) {
        self.terminal_tab_collection.close_tab(tab_id);
    }

    fn move_tab(&mut self, from: usize, to: usize) {
        // TODO implement moving a tab from an index to another
    }

    // fn activate_next_tab(&mut selt) {
        // TODO implement activating the next tab
    // }

    // fn activate_previous_tab(&mut selt) {
        // TODO implement activating the previous tab
    // }

    fn change_font_size(&mut self, delta: f32) {
        *self.font_size = max(*self.font_size + delta, Size::new(FONT_SIZE_STEP));
        let font = self.config.font.clone().with_size(*self.font_size);
        self.display_update_pending.font = Some(font);
        self.terminal.dirty = true;
    }

    fn reset_font_size(&mut self) {
        *self.font_size = self.config.font.size;
        self.display_update_pending.font = Some(self.config.font.clone());
        self.terminal.dirty = true;
    }

    fn pop_message(&mut self) {
        self.display_update_pending.message_buffer = Some(());
        self.message_buffer.pop();
    }

    fn message(&self) -> Option<&Message> {
        self.message_buffer.message()
    }

    fn config(&self) -> &Config {
        self.config
    }
}

pub enum ClickState {
    None,
    Click,
    DoubleClick,
    TripleClick,
}

/// State of the mouse
pub struct Mouse {
    pub x: usize,
    pub y: usize,
    pub left_button_state: ElementState,
    pub middle_button_state: ElementState,
    pub right_button_state: ElementState,
    pub last_click_timestamp: Instant,
    pub click_state: ClickState,
    pub scroll_px: i32,
    pub line: Line,
    pub column: Column,
    pub cell_side: Side,
    pub lines_scrolled: f32,
    pub block_url_launcher: bool,
    pub last_button: MouseButton,
    pub inside_grid: bool,
}

impl Default for Mouse {
    fn default() -> Mouse {
        Mouse {
            x: 0,
            y: 0,
            last_click_timestamp: Instant::now(),
            left_button_state: ElementState::Released,
            middle_button_state: ElementState::Released,
            right_button_state: ElementState::Released,
            click_state: ClickState::None,
            scroll_px: 0,
            line: Line(0),
            column: Column(0),
            cell_side: Side::Left,
            lines_scrolled: 0.0,
            block_url_launcher: false,
            last_button: MouseButton::Other(0),
            inside_grid: false,
        }
    }
}

/// The event processor
///
/// Stores some state from received events and dispatches actions when they are
/// triggered.
pub struct Processor {
    display_context_map: DisplayContextMap,
    mouse: Mouse,
    received_count: usize,
    suppress_chars: bool,
    modifiers: ModifiersState,
    config: Config,
    message_buffer: MessageBuffer,
    font_size: Size,
}

impl Processor {
    /// Create a new event processor
    ///
    /// Takes a writer which is expected to be hooked up to the write end of a
    /// pty.
    pub fn new(
        display_context_map: DisplayContextMap,
        message_buffer: MessageBuffer,
        config: Config,
    ) -> Processor {
        Processor {
            display_context_map,
            mouse: Default::default(),
            received_count: 0,
            suppress_chars: false,
            modifiers: Default::default(),
            font_size: config.font.size,
            config,
            message_buffer,
        }
    }

    /// Run the event loop.
    pub fn run(&mut self, mut window_event_loop: EventLoop<Event>, event_proxy: &EventProxy) {
        let mut event_queue = Vec::new();
        let mut pending_display_context_refesh = false;

        window_event_loop.run_return(|event, event_loop, control_flow| {   
            if self.config.debug.print_events {
                info!("glutin event: {:?}", event);
            }
            
            let mut is_close_requested = false;
            let mut is_user_exit = false;
            let mut display_command_queue = DisplayCommandQueue::default();

            // Activation & Deactivation of windows           
            if let GlutinEvent::WindowEvent { event, window_id, .. } = &event {
                use glutin::event::WindowEvent::*;
                match &event{
                    Focused(is_focused) => {
                        if *is_focused {
                            self.display_context_map.activate_window(*window_id);
                        } else {
                            self.display_context_map.deactivate_window(*window_id);
                        }
                    },
                    CloseRequested => {
                        is_close_requested = true;
                        let display_ctx = self.display_context_map.get_active_display_context();
                        let display_arc = display_ctx.display.clone();
                        let display = display_arc.lock();
                        let window = &display.window;
                        window.close();
                        self.display_context_map.exit(*window_id);
                    }
                    _ => {}
                }
            };

            if self.display_context_map.is_empty() {
                *control_flow = ControlFlow::Exit;
                return;
            }

            if !self.display_context_map.has_active_display() {
                return;
            }
            
            match &event {
                // Check for shutdown
                GlutinEvent::UserEvent(Event::Exit) => {
                    is_user_exit = true;
                },

                // Process events
                GlutinEvent::EventsCleared => {
                    *control_flow = ControlFlow::Wait;

                    if event_queue.is_empty() {
                        return;
                    }
                },

                // Buffer events
                _ => {
                    *control_flow = ControlFlow::Poll;
                    if !Self::skip_event(&event) {
                        event_queue.push(event);
                    }
                    return;
                },
            }

            let display_ctx = self.display_context_map.get_active_display_context();
            let term_tab_collection_arc = display_ctx.term_tab_collection.clone();
            let mut term_tab_collection = term_tab_collection_arc.lock();
            let display_arc = display_ctx.display.clone();
            let mut display = display_arc.lock();

            if term_tab_collection.is_empty() {
                let window = &display.window;
                self.display_context_map.exit(window.window_id());
                window.close();
                return;
            }

            // handle pty detach
            if !is_close_requested && is_user_exit {
                term_tab_collection.close_current_tab();
                term_tab_collection.commit_changes(&self.config, display.size_info);
                return;
            }

            let active_term_mutex = term_tab_collection.get_active_tab().clone();
            let mut terminal_ctx = active_term_mutex.lock();
            let terminal_arc = terminal_ctx.terminal.clone();
            let mut terminal = terminal_arc.lock();

            let mut display_update_pending = DisplayUpdate::default();
            
            let mut size_info = display.size_info;
            let urls = display.urls.clone();
            let highlighted_url = display.highlighted_url.clone();

            let context = ActionContext {
                display_command_queue: &mut display_command_queue,
                display_context_map: &mut self.display_context_map,
                terminal_tab_collection: &mut term_tab_collection,
                terminal: &mut terminal,
                notifier: terminal_ctx.notifier.as_mut(),
                mouse: &mut self.mouse,
                size_info: &mut size_info,
                received_count: &mut self.received_count,
                suppress_chars: &mut self.suppress_chars,
                modifiers: &mut self.modifiers,
                message_buffer: &mut self.message_buffer,
                display_update_pending: &mut display_update_pending,
                window: &mut display.window,
                font_size: &mut self.font_size,
                config: &mut self.config,
            };

            let mut processor =
                 input::Processor::new(context, &urls, &highlighted_url);

            for event in event_queue.drain(..) {
                Processor::handle_event(event, &mut processor);
            }
            
            if term_tab_collection.is_empty() {
                return;
            }
            
            let did_create_new_display = self.display_context_map.is_pending_create_display();

            match self.display_context_map.commit_changes(
                &mut display_command_queue,
                size_info,
                &mut term_tab_collection,
                &self.config, 
                &event_loop, 
                event_proxy,
            ) {
                Ok(is_dirty) => is_dirty,
                Err(_error) => return,
            };

            // Process resize events
            if !display_update_pending.is_empty() {
                display.handle_update(
                    &mut terminal,
                    terminal_ctx.resize_handle.as_mut(),
                    &self.message_buffer,
                    &self.config,
                    display_update_pending,
                );
            }

            if terminal.dirty || did_create_new_display {
                terminal.dirty = false;

                // Request immediate re-draw if visual bell animation is not finished yet
                if !terminal.visual_bell.completed() {
                    event_queue.push(GlutinEvent::UserEvent(Event::Wakeup));
                }

                // Redraw screen
                display.draw(
                    terminal,
                    &self.message_buffer,
                    &self.config,
                    &self.mouse,
                    self.modifiers,
                );
            }
        });

        // Write ref tests to disk
        // TODO - Loop through all the terminals in the terminal collection and write refs to disk
        // self.write_ref_test_results(&terminal.lock());
    }

    /// Handle events from glutin
    ///
    /// Doesn't take self mutably due to borrow checking.
    fn handle_event<T: 'static + Clone + Send>(
        event: GlutinEvent<Event>,
        processor: &mut input::Processor<T, ActionContext<Notifier, T>>,
    ) where
        T: EventListener,
    {
        match event {
            GlutinEvent::UserEvent(event) => match event {
                Event::Title(title) => processor.ctx.window.set_title(&title),
                Event::Wakeup => processor.ctx.terminal.dirty = true,
                Event::Urgent => {
                    processor.ctx.window.set_urgent(!processor.ctx.terminal.is_focused)
                },
                Event::ConfigReload(path) => {
                    processor.ctx.message_buffer.remove_target(LOG_TARGET_CONFIG);
                    processor.ctx.display_update_pending.message_buffer = Some(());

                    if let Ok(config) = config::reload_from(&path) {
                        processor.ctx.terminal.update_config(&config);

                        if processor.ctx.config.font != config.font {
                            // Do not update font size if it has been changed at runtime
                            if *processor.ctx.font_size == processor.ctx.config.font.size {
                                *processor.ctx.font_size = config.font.size;
                            }

                            let font = config.font.clone().with_size(*processor.ctx.font_size);
                            processor.ctx.display_update_pending.font = Some(font);
                        }

                        *processor.ctx.config = config;

                        processor.ctx.terminal.dirty = true;
                    }
                },
                Event::Message(message) => {
                    processor.ctx.message_buffer.push(message);
                    processor.ctx.display_update_pending.message_buffer = Some(());
                    processor.ctx.terminal.dirty = true;
                },
                Event::MouseCursorDirty => processor.reset_mouse_cursor(),
                Event::Exit => (),
            },
            GlutinEvent::WindowEvent { event, window_id, .. } => {
                use glutin::event::WindowEvent::*;
                match event {
                    CloseRequested => {
                        // This is handled as part of the main loop now as part of the window activation/deactivation
                    },
                    Resized(lsize) => {
                        let psize = lsize.to_physical(processor.ctx.size_info.dpr);
                        processor.ctx.display_update_pending.dimensions = Some(psize);
                        processor.ctx.terminal.dirty = true;
                    },
                    KeyboardInput { input, .. } => {
                        processor.key_input(input);
                        if input.state == ElementState::Pressed {
                            // Hide cursor while typing
                            if processor.ctx.config.ui_config.mouse.hide_when_typing {
                                processor.ctx.window.set_mouse_visible(false);
                            }
                        }
                    },
                    ReceivedCharacter(c) => processor.received_char(c),
                    MouseInput { state, button, modifiers, .. } => {
                        if !cfg!(target_os = "macos") || processor.ctx.terminal.is_focused {
                            processor.ctx.window.set_mouse_visible(true);
                            processor.mouse_input(state, button, modifiers);
                            processor.ctx.terminal.dirty = true;
                        }
                    },
                    CursorMoved { position: lpos, modifiers, .. } => {
                        let (x, y) = lpos.to_physical(processor.ctx.size_info.dpr).into();
                        let x: i32 = limit(x, 0, processor.ctx.size_info.width as i32);
                        let y: i32 = limit(y, 0, processor.ctx.size_info.height as i32);

                        processor.ctx.window.set_mouse_visible(true);
                        processor.mouse_moved(x as usize, y as usize, modifiers);
                    },
                    MouseWheel { delta, phase, modifiers, .. } => {
                        processor.ctx.window.set_mouse_visible(true);
                        processor.mouse_wheel_input(delta, phase, modifiers);
                    },
                    Focused(is_focused) => {
                        if window_id == processor.ctx.window.window_id() {
                            processor.ctx.terminal.is_focused = is_focused;
                            processor.ctx.terminal.dirty = true;

                            if is_focused {
                                processor.ctx.window.set_urgent(false);
                            } else {
                                processor.ctx.window.set_mouse_visible(true);
                            }

                            processor.on_focus_change(is_focused);
                        }
                    },
                    DroppedFile(path) => {
                        // TODO not sure why I need to import the ActionContext here
                        use crate::input::ActionContext;
                        let path: String = path.to_string_lossy().into();
                        processor.ctx.write_to_pty(path.into_bytes());
                    },
                    HiDpiFactorChanged(dpr) => {
                        let dpr_change = (dpr / processor.ctx.size_info.dpr) as f32;
                        let display_update_pending = &mut processor.ctx.display_update_pending;

                        // Push current font to update its DPR
                        display_update_pending.font = Some(processor.ctx.config.font.clone());

                        // Scale window dimensions with new DPR
                        let old_width = processor.ctx.size_info.width;
                        let old_height = processor.ctx.size_info.height;
                        let dimensions =
                            display_update_pending.dimensions.get_or_insert_with(|| {
                                PhysicalSize::new(f64::from(old_width), f64::from(old_height))
                            });
                        dimensions.width *= f64::from(dpr_change);
                        dimensions.height *= f64::from(dpr_change);

                        processor.ctx.terminal.dirty = true;
                        processor.ctx.size_info.dpr = dpr;
                    },
                    RedrawRequested => processor.ctx.terminal.dirty = true,
                    CursorLeft { .. } => {
                        processor.ctx.mouse.inside_grid = false;

                        if processor.highlighted_url.is_some() {
                            processor.ctx.terminal.dirty = true;
                        }
                    },
                    TouchpadPressure { .. }
                    | CursorEntered { .. }
                    | AxisMotion { .. }
                    | HoveredFileCancelled
                    | Destroyed
                    | HoveredFile(_)
                    | Touch(_)
                    | Moved(_) => (),
                }
            },
            GlutinEvent::DeviceEvent { event, .. } => {
                use glutin::event::DeviceEvent::*;
                if let ModifiersChanged { modifiers } = event {
                    processor.modifiers_input(modifiers);
                }
            },
            GlutinEvent::Suspended { .. }
            | GlutinEvent::NewEvents { .. }
            | GlutinEvent::EventsCleared
            | GlutinEvent::Resumed
            | GlutinEvent::LoopDestroyed => (),
        }
    }

    /// Check if an event is irrelevant and can be skipped
    fn skip_event(event: &GlutinEvent<Event>) -> bool {
        match event {
            GlutinEvent::UserEvent(Event::Exit) => true,
            GlutinEvent::WindowEvent { event, .. } => {
                use glutin::event::WindowEvent::*;
                match event {
                    TouchpadPressure { .. }
                    | CursorEntered { .. }
                    | AxisMotion { .. }
                    | HoveredFileCancelled
                    | Destroyed
                    | HoveredFile(_)
                    | Touch(_)
                    | Moved(_) => true,
                    _ => false,
                }
            },
            GlutinEvent::DeviceEvent { event, .. } => {
                use glutin::event::DeviceEvent::*;
                match event {
                    ModifiersChanged { .. } => false,
                    _ => true,
                }
            },
            GlutinEvent::Suspended { .. }
            | GlutinEvent::NewEvents { .. }
            | GlutinEvent::EventsCleared
            | GlutinEvent::LoopDestroyed => true,
            _ => false,
        }
    }

    // TODO put this function back
    // Write the ref test results to the disk
    // pub fn write_ref_test_results<T>(&self, terminal: &Term<T>) {
    //     if !self.config.debug.ref_test {
    //         return;
    //     }

    //     // dump grid state
    //     let mut grid = terminal.grid().clone();
    //     grid.initialize_all(&Cell::default());
    //     grid.truncate();

    //     let serialized_grid = json::to_string(&grid).expect("serialize grid");

    //     let serialized_size = json::to_string(&self.display.size_info).expect("serialize size");

    //     let serialized_config = format!("{{\"history_size\":{}}}", grid.history_size());

    //     File::create("./grid.json")
    //         .and_then(|mut f| f.write_all(serialized_grid.as_bytes()))
    //         .expect("write grid.json");

    //     File::create("./size.json")
    //         .and_then(|mut f| f.write_all(serialized_size.as_bytes()))
    //         .expect("write size.json");

    //     File::create("./config.json")
    //         .and_then(|mut f| f.write_all(serialized_config.as_bytes()))
    //         .expect("write config.json");
    // }
}

#[derive(Debug, Clone)]
pub struct EventProxy(EventLoopProxy<Event>);

impl EventProxy {
    pub fn new(proxy: EventLoopProxy<Event>) -> Self {
        EventProxy(proxy)
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.0.send_event(event);
    }
}
