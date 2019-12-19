use glutin::window::WindowId;

pub(super) struct MultiWindowPlatform {
    platform_impl: Box<dyn MultiWindowPlatformImpl>,
} 

impl MultiWindowPlatform {
    pub fn new(window_id: WindowId) -> MultiWindowPlatform {
        MultiWindowPlatform {
            #[cfg(target_os = "macos")]
            platform_impl: Box::new(MultiWindowPlatformMacOs::new(window_id))
        }
    }

    pub fn initialize(&mut self) {
        self.platform_impl.initialize();
    }
}

trait MultiWindowPlatformImpl {
    fn initialize(&mut self);
}

struct MultiWindowPlatformMacOs {
    window_id: WindowId,
}

impl MultiWindowPlatformImpl for MultiWindowPlatformMacOs {
    fn initialize(&mut self) {
        println!("handle cascading...");
        self.handle_macos_window_cascading();
    }
}

impl MultiWindowPlatformMacOs {
    fn new(window_id: WindowId) -> MultiWindowPlatformMacOs {
        MultiWindowPlatformMacOs {
            window_id
        }
    }

    fn handle_macos_window_cascading(&self) {
        use objc::{ msg_send, sel, sel_impl };
        use cocoa::{ base::{id, nil}, foundation::{NSPoint}};

        unsafe {
            let shared_application = cocoa::appkit::NSApplication::sharedApplication(nil);
            let windows: id = msg_send![shared_application,  windows];

            let main_window: id = msg_send![shared_application,  mainWindow];
            let ns_point: NSPoint = msg_send![main_window, cascadeTopLeftFromPoint: NSPoint {x: 0.0, y: 0.0}];

            let window: id = msg_send![windows, lastObject];
            let _result: id = msg_send![window, cascadeTopLeftFromPoint: ns_point];   
        }
    }
}
