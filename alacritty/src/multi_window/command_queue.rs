use mio_extras::channel::Sender;

use glutin::event_loop::EventLoopWindowTarget;
use glutin::window::WindowId;

use crate::display;
use crate::config::Config;
use alacritty_terminal::event::Event;

use crate::event::EventProxy;


use crate::multi_window::term_tab::MultiWindowEvent;
use crate::multi_window::term_tab_collection::TermTabCollection;
use crate::multi_window::window_context_tracker::WindowContext;
use crate::multi_window::window_context_tracker::WindowContextTracker;

#[derive(Clone, PartialEq)]
pub enum MultiWindowCommand {
	CreateWindow,
	ActivateWindow(WindowId), // WindowId
	DeactivateWindow(WindowId), // WindowId
	CloseWindow(WindowId), // WindowId
	CreateTab(WindowId), // WindowId
	MoveTab(WindowId, usize, usize), // WindowId, tab_id, new_index
	SetTabTitle(WindowId, usize, String), // WindowId, tab_id, new title
	ActivateTab(WindowId, usize), // tab_id
	CloseCurrentTab(WindowId), // WindowId
	CloseTab(WindowId, usize), // tab_id
}

#[derive(Default)]
pub struct MultiWindowCommandQueue {
	queue: Vec<MultiWindowCommand>,
	has_create: bool,
}

impl MultiWindowCommandQueue {
	pub fn push(&mut self, command: MultiWindowCommand) {
		if command == MultiWindowCommand::CreateWindow {
			self.has_create = true;
		}

		self.queue.push(command);
	}

	// TODO reduce function complexity
	pub fn run<'a>(
		&mut self,
		context_tracker: &mut WindowContextTracker,
		config: &Config,
		window_event_loop: &EventLoopWindowTarget<Event>,
		event_proxy: &EventProxy,
		dispatcher: Sender<MultiWindowEvent>,
	) -> Result<(), display::Error> {
		for command in self.queue.drain(..) {
			match command {
				MultiWindowCommand::CreateWindow => {
					context_tracker.create_window_context(
						&config,
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
						let mut tab_collection = window_ctx.term_tab_collection.lock();

						let tab_id = tab_collection.add_tab(
							&config, 
							size_info, 
							Some(window_ctx.window_id), 
							&dispatcher
						);

						tab_collection.activate_tab(tab_id);

						update_size(&window_ctx, &tab_collection, config);
					}
				}

				MultiWindowCommand::MoveTab(window_id, tab_id, new_tab_id) => {
					println!("Move tab called on window {:?} from {} to {}", window_id, tab_id, new_tab_id);
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						let mut tab_collection = window_ctx.term_tab_collection.lock();
						tab_collection.move_tab(tab_id, new_tab_id);
					}
				}

				MultiWindowCommand::ActivateTab(window_id, tab_id) => {
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						let mut tab_collection = window_ctx.term_tab_collection.lock();
						tab_collection.activate_tab(tab_id);
						window_ctx.processor.lock().request_redraw();
					}
				}

				MultiWindowCommand::CloseCurrentTab(window_id) => {
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						let mut tab_collection = window_ctx.term_tab_collection.lock();
						tab_collection.close_current_tab();
						if tab_collection.is_empty() {
							context_tracker.close_window(window_ctx.window_id);
						} else {
							update_size(&window_ctx, &tab_collection, config);
						}
					}
				}

				MultiWindowCommand::CloseTab(window_id, tab_id) => {
					if let Some(window_ctx) = context_tracker.get_context(window_id) {
						let mut tab_collection = window_ctx.term_tab_collection.lock();
						tab_collection.close_tab(tab_id);

						if tab_collection.is_empty() {
							context_tracker.close_window(window_ctx.window_id);
						} else {
							update_size(&window_ctx, &tab_collection, config);
						}
					};
				}
			};
		}

		Ok(())
	}
}

fn update_size(window_ctx: &WindowContext, tab_collection: &TermTabCollection<EventProxy>, config: &Config) {
	let active_tab = tab_collection.active_tab().unwrap();
	let mut processor = window_ctx.processor.lock();
	processor.update_size(&mut active_tab.terminal.lock(), config);
	processor.request_redraw();
}
