//! Real surface tests opt in via DISPLAY (run under Xvfb on headless CI).
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, RawWindowHandle,
};
use std::sync::{Arc, Mutex, MutexGuard};
use x11_dl::xlib;

static LOCK: Mutex<()> = Mutex::new(());
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct TestWindow {
    xlib: xlib::Xlib,
    display: *mut xlib::Display,
    window: xlib::Window,
    screen: i32,
}

impl std::fmt::Debug for TestWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestWindow")
    }
}

// The real owner is shared, all Xlib use is serialized by LOCK, and XInitThreads
// runs before opening the display. No production raw-handle wrapper is involved.
unsafe impl Send for TestWindow {}
unsafe impl Sync for TestWindow {}

impl TestWindow {
    pub(crate) fn open() -> Option<Arc<Self>> {
        if std::env::var_os("DISPLAY").is_none() {
            assert!(
                std::env::var_os("NERUST_REQUIRE_GPU_TESTS").is_none(),
                "GPU tests require DISPLAY"
            );
            eprintln!("SKIP real wgpu surface: DISPLAY unset (use xvfb-run)");
            return None;
        }
        let xlib = xlib::Xlib::open().expect("Xlib");
        unsafe {
            assert_ne!((xlib.XInitThreads)(), 0);
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            assert!(!display.is_null(), "XOpenDisplay");
            let screen = (xlib.XDefaultScreen)(display);
            let root = (xlib.XRootWindow)(display, screen);
            let window = (xlib.XCreateSimpleWindow)(display, root, 0, 0, 128, 128, 0, 0, 0);
            assert_ne!(window, 0);
            (xlib.XMapWindow)(display, window);
            (xlib.XSync)(display, 0);
            Some(Arc::new(Self {
                xlib,
                display,
                window,
                screen,
            }))
        }
    }

    pub(crate) fn handles(&self) -> (RawWindowHandle, RawDisplayHandle) {
        (
            RawWindowHandle::Xlib(raw_window_handle::XlibWindowHandle::new(self.window)),
            RawDisplayHandle::Xlib(raw_window_handle::XlibDisplayHandle::new(
                std::ptr::NonNull::new(self.display.cast()),
                self.screen,
            )),
        )
    }
}

impl HasDisplayHandle for TestWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        unsafe { Ok(DisplayHandle::borrow_raw(self.handles().1)) }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}
