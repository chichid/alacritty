use alacritty_terminal::event::Event;
use alacritty_terminal::sync::FairMutex;
use mio_extras::channel::Sender;
use std::sync::Arc;

use glutin::event_loop::EventLoopWindowTarget;
use glutin::window::WindowId;

use crate::config::Config;
use crate::display;
use crate::multi_window::term_tab::MultiWindowEvent;
use crate::multi_window::window_context_tracker::WindowContext;

use crate::event::EventProxy;
use crate::multi_window::window_context_tracker::WindowContextTracker;

#[derive(Clone, PartialEq)]
pub enum MultiWindowCommand {
	NewWindow,
	ActivateWindow(WindowId),
	DeactivateWindow(WindowId),
	CloseWindow(WindowId),
	CreateTab(WindowId),
	SetTabTitle(WindowId, usize, String),
	ActivateTab(usize), // tab_id
	CloseCurrentTab,
	CloseTab(usize), // tab_id
}

#[derive(Default)]
pub struct MultiWindowCommandQueue {
	queue: Vec<MultiWindowCommand>,
	has_create: bool,
}

impl MultiWindowCommandQueue {
	pub fn push(&mut self, command: MultiWindowCommand) {
		if command == MultiWindowCommand::NewWindow {
			self.has_create = true;
		}

		self.queue.push(command);
	}

	pub fn run<'a>(
		&mut self,
		context_tracker: &mut WindowContextTracker,
		config: &'a mut Config,
		window_event_loop: &EventLoopWindowTarget<Event>,
		event_proxy: &EventProxy,
		dispatcher: Sender<MultiWindowEvent>,
	) -> Result<(), display::Error> {
		let config_arc = Arc::new(FairMutex::new(config));
		let need_redraw = !self.queue.is_empty();

		for command in self.queue.drain(..) {
			match command {
				MultiWindowCommand::NewWindow => {
					let mut config = config_arc.lock();
					context_tracker.create_window_context(
						&mut config,
						window_event_loop,
						event_proxy,
						dispatcher.clone(),
					)?;
				}

				MultiWindowCommand::ActivateWindow(window_id) => {
					context_tracker.activate_window(window_id);
				}

				MultiWindowCommand::DeactivateWindow(window_id) => {
					context_tracker.deactivate_window(window_id);
				}

				MultiWindowCommand::CloseWindow(window_id) => {
					context_tracker.close_window(window_id);
				}

				MultiWindowCommand::SetTabTitle(window_id, tab_id, title) => {
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						window_ctx.term_tab_collection.lock().tab_mut(tab_id).set_title(title);
						window_ctx.processor.lock().request_redraw();
					}
				}

				MultiWindowCommand::CreateTab(window_id) => {
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						let size_info = window_ctx.processor.lock().get_size_info();
						let config = config_arc.lock();
						let mut tab_collection = window_ctx.term_tab_collection.lock();

						let tab_id =
							tab_collection.add_tab(&config, size_info, Some(window_ctx.window_id), &dispatcher);

						tab_collection.activate_tab(tab_id);
					}
				}

				MultiWindowCommand::ActivateTab(tab_id) => {
					let window_ctx = context_tracker.get_active_window_context();
					let mut tab_collection = window_ctx.term_tab_collection.lock();
					tab_collection.activate_tab(tab_id);
				}

				MultiWindowCommand::CloseCurrentTab => {
					let window_ctx = context_tracker.get_active_window_context();
					let mut tab_collection = window_ctx.term_tab_collection.lock();
					tab_collection.close_current_tab();

					if tab_collection.is_empty() {
						context_tracker.close_window(window_ctx.window_id);
					}
				}

				MultiWindowCommand::CloseTab(tab_id) => {
					let window_ctx = context_tracker.get_active_window_context();
					let mut tab_collection = window_ctx.term_tab_collection.lock();
					tab_collection.close_tab(tab_id);

					if tab_collection.is_empty() {
						context_tracker.close_window(window_ctx.window_id);
					}
				}
			};
		}

		if need_redraw && context_tracker.has_active_window() {
			let window_ctx = context_tracker.get_active_window_context();

			let terminal = {
				let tab_collection = window_ctx.term_tab_collection.lock();
				let active_tab = tab_collection.active_tab().unwrap();
				active_tab.terminal
			};

			let mut processor = window_ctx.processor.lock();
			let config = config_arc.lock();
			processor.update_size(&mut terminal.lock(), &config);
			processor.request_redraw();
		}

		Ok(())
	}
}
