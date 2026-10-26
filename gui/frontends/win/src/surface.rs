//! Win32 surface seam: the only `HWND` knowledge a native port needs.
//!
//! Production mapping (left for the real port):
//! - `set_title` → `SetWindowTextW`
//! - `set_fullscreen` → borderless `SetWindowPos` over the monitor rect
//! - `pick_rom_file` → `GetOpenFileNameW` with the host-provided filters
//! - `show_error_modal` → `MessageBoxW` (`MB_ICONERROR`, owner `HWND`)
//! - `request_debugger` → `CreateWindowExW` for the debugger class
//! - `run_frame` has no Win32 call: the message loop calls `WinHost::turn`
//!   per `WM_PAINT`/timer, mirroring the tao burst-end commit.

use std::path::PathBuf;

/// Native window effects the host returns. One method per effect keeps
/// the seam auditable; a new effect is a compile error here, not a
/// silent drop in a message handler.
pub trait WinSurface {
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

impl WinSurface for HeadlessSurface {
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

/// Real-port anchor: each method maps to exactly one Win32 call.
/// Left `todo!()` so the seam compiles on every platform; the Linux
/// prototype never instantiates this type.
#[cfg(target_os = "windows")]
pub struct Win32Surface {
    // hwnd: isize,
}

#[cfg(target_os = "windows")]
impl WinSurface for Win32Surface {
    fn set_title(&mut self, _title: &str) {
        todo!("SetWindowTextW")
    }

    fn set_fullscreen(&mut self, _fullscreen: bool) {
        todo!("borderless SetWindowPos over the monitor rect")
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        todo!("GetOpenFileNameW")
    }

    fn show_error_modal(&mut self, _detail: &str) {
        todo!("MessageBoxW with MB_ICONERROR")
    }

    fn request_debugger(&mut self) {
        todo!("CreateWindowExW for the debugger class")
    }
}
