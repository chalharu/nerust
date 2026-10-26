//! Fluent chrome for the Win32 demo: dark title bars, rounded corners,
//! Mica backdrop, dark Explorer theming, and Segoe UI typography through
//! plain windows-rs DWM/uxtheme calls (no WinUI/XAML dependency, so the
//! exe still runs under Wine — unsupported attributes are ignored).
//!
//! Policy stays in `nerust_frontend_host`; this module only styles HWNDs.

use std::sync::OnceLock;

use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        UI::{Controls::*, WindowsAndMessaging::*},
    },
    core::*,
};

/// Fluent dark card: window/dialog background.
pub const FLUENT_BG: u32 = 0x0020_2020;
/// Primary text on dark surfaces.
pub const FLUENT_TEXT: u32 = 0x00F3_F3F3;
/// Window border line.
pub const FLUENT_BORDER: u32 = 0x003A_3A3A;

fn set_attr(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: &[u8]) {
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            attr,
            value.as_ptr() as *const std::ffi::c_void,
            value.len() as u32,
        );
    }
}

/// Dark title bar, rounded corners, and Mica backdrop. Best effort: every
/// call is fallible by design so Wine (which stubs most of DWM) keeps
/// working with classic chrome.
pub fn apply_chrome(hwnd: HWND) {
    let on: i32 = 1;
    let on = on.to_ne_bytes();
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on);
    set_attr(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &DWMWCP_ROUND.0.to_ne_bytes(),
    );
    set_attr(
        hwnd,
        DWMWA_SYSTEMBACKDROP_TYPE,
        &DWMSBT_MAINWINDOW.0.to_ne_bytes(),
    );
    set_attr(hwnd, DWMWA_CAPTION_COLOR, &FLUENT_BG.to_ne_bytes());
    set_attr(hwnd, DWMWA_TEXT_COLOR, &FLUENT_TEXT.to_ne_bytes());
    set_attr(hwnd, DWMWA_BORDER_COLOR, &FLUENT_BORDER.to_ne_bytes());
    // Dark scrollbars, dark context menus, dark tooltip surfaces where
    // the OS theme engine supports them.
    unsafe {
        let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR::null());
    }
}

/// Process message font (Segoe UI 9pt on stock Windows 10/11). Cached for
/// the process lifetime; windows share the stock-derived HFONT.
pub fn ui_font() -> HFONT {
    static FONT: OnceLock<HFONT> = OnceLock::new();
    *FONT.get_or_init(|| unsafe {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let ok = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        if ok.is_ok() {
            CreateFontIndirectW(&metrics.lfMessageFont)
        } else {
            HFONT::default()
        }
    })
}

/// Set the Fluent body font on a child control. No-op when the stock font
/// lookup failed.
pub fn apply_font(hwnd: HWND) {
    let font = ui_font();
    if font != HFONT::default() {
        unsafe {
            SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
    }
}

/// Dark card brush for window-class backgrounds and `WM_CTLCOLOR*`.
pub fn dark_brush() -> HBRUSH {
    static BRUSH: OnceLock<HBRUSH> = OnceLock::new();
    *BRUSH.get_or_init(|| unsafe { CreateSolidBrush(COLORREF(FLUENT_BG)) })
}

/// Shared `WM_CTLCOLORSTATIC/LISTBOX/EDIT` handler: light Fluent text on
/// the dark card. Returns the brush to keep (process lifetime).
pub fn paint_dark_control(hdc: HDC) -> HBRUSH {
    unsafe {
        SetTextColor(hdc, COLORREF(FLUENT_TEXT));
        SetBkColor(hdc, COLORREF(FLUENT_BG));
        let _ = SetBkMode(hdc, TRANSPARENT);
    }
    dark_brush()
}
