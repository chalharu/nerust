//! AppKit surface seam: the only `NSWindow` knowledge a native port needs.
//!
//! Production mapping (left for the real port):
//! - `set_title` → `[NSWindow setTitle:]`
//! - `set_fullscreen` → `[NSWindow toggleFullScreen:]`
//! - `pick_rom_file` → `NSOpenPanel` with the host-provided `allowedFileTypes`
//! - `show_error_modal` → `NSAlert` `beginSheetModalForWindow:`
//! - `request_debugger` → `NSPanel` for the debugger class
//! - the display-link callback calls `MacHost::turn`, mirroring the
//!   tao burst-end commit.

use std::path::PathBuf;

/// Native window effects the host returns. One method per effect keeps
/// the seam auditable; a new effect is a compile error here, not a
/// silent drop in a selector.
pub trait MacSurface {
    fn set_title(&mut self, title: &str);
    fn set_fullscreen(&mut self, fullscreen: bool);
    fn pick_rom_file(&mut self, filters: &[(String, Vec<String>)]) -> Option<PathBuf>;
    fn show_error_modal(&mut self, detail: &str);
    fn request_debugger(&mut self);
}

/// Headless fake for tests: records every call, answers from scripted state.
#[derive(Debug, Default)]
pub struct HeadlessSurface {
    pub titles: Vec<String>,
    pub fullscreen_calls: Vec<bool>,
    pub modal_errors: Vec<String>,
    pub debugger_requests: u32,
    pub rom_pick: Option<PathBuf>,
}

impl MacSurface for HeadlessSurface {
    fn set_title(&mut self, title: &str) {
        self.titles.push(title.to_string());
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen_calls.push(fullscreen);
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        self.rom_pick.clone()
    }

    fn show_error_modal(&mut self, detail: &str) {
        self.modal_errors.push(detail.to_string());
    }

    fn request_debugger(&mut self) {
        self.debugger_requests += 1;
    }
}

/// Real-port anchor: each method maps to exactly one AppKit call.
/// Left `todo!()` so the seam compiles on every platform; the Linux
/// prototype never instantiates this type.
#[cfg(target_os = "macos")]
pub struct AppKitSurface {
    // window: *mut libc::c_void,
}

#[cfg(target_os = "macos")]
impl MacSurface for AppKitSurface {
    fn set_title(&mut self, _title: &str) {
        todo!("[NSWindow setTitle:]")
    }

    fn set_fullscreen(&mut self, _fullscreen: bool) {
        todo!("[NSWindow toggleFullScreen:]")
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        todo!("NSOpenPanel with allowedFileTypes")
    }

    fn show_error_modal(&mut self, _detail: &str) {
        todo!("NSAlert beginSheetModalForWindow:")
    }

    fn request_debugger(&mut self) {
        todo!("NSPanel for the debugger class")
    }
}
