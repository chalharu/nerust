//! Prototype Win32-native frontend over the shared frontend host.
//!
//! Scope: prove a third native frontend can be built thin — policy and
//! ordering live in `nerust_frontend_host`, this crate only translates
//! Win32-shaped inputs (virtual-key codes, `WM_COMMAND` ids, focus
//! notifications, per-frame ticks) into host events and executes the
//! returned native effects through [`WinSurface`].
//!
//! Real window creation (`CreateWindowEx`, `GetOpenFileNameW`,
//! `MessageBoxW`, `SetWindowText`) is intentionally absent: the
//! `#[cfg(target_os = "windows")]` [`Win32Surface`] stub marks exactly
//! where each `HWND` call plugs in. Everything else — dispatch gates,
//! settings staging, focus protocol, loop ordering — is shared and
//! covered by the headless tests in this crate.

pub mod host;
pub mod keys;
pub mod surface;

pub use host::WinHost;
pub use keys::{win_key_to_nerust, win_menu_event};
pub use surface::{HeadlessSurface, WinSurface};
