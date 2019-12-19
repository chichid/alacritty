use glutin::window::WindowId;

pub(super) struct TabView {
    platform_tabview: Box<dyn PlatformTabview>,
} 

trait PlatformTabview {
    fn initialize(&self);
}

impl TabView {
    pub fn new(window_id: WindowId) -> TabView {
        TabView {
            #[cfg(target_os = "macos")]
            platform_tabview: Box::new(TabViewMacOs::new(window_id))
        }
    }
}

struct TabViewMacOs {
    window_id: WindowId,
}

impl TabViewMacOs {
    fn new(window_id: WindowId) -> TabViewMacOs {
        TabViewMacOs {
            window_id
        }
    }
}

impl PlatformTabview for TabViewMacOs {
    fn initialize(&self) {
    }
}