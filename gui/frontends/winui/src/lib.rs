//! Prototype WinUI 3-native frontend over the shared frontend host.
//!
//! A WinUI 3 port of the Win32 demo: windows, menus, settings, debugger,
//! and PPU viewer are code-built XAML (no XAML compiler runs on this
//! machine), while every policy decision stays in
//! `nerust_frontend_host`. The Win32-shaped dispatch layer is reused
//! wholesale from `nerust_win` (`WinHost`, `keys::menu`, `WinSurface`):
//! WinUI menu items and key events carry the same ids/codes, and native
//! effects execute through a XAML-backed [`surface::WinUiSurface`].
//!
//! Bindings in [`bindings`] were generated offline with
//! `windows-bindgen 0.62.1` against WindowsAppSDK 1.4 winmd plus the
//! bindgen-bundled Windows metadata (see README for the exact command).
//! The app targets the 1.4 framework (forward compatible with newer 1.x
//! runtimes); unpackaged deployment calls `MddBootstrapInitialize` in
//! `main` before `Application::Start`.

#[cfg(windows)]
pub mod bindings;
#[cfg(windows)]
pub mod boot;
pub mod surface;
#[cfg(windows)]
pub mod ui;

/// Unpackaged entry point: bootstrap the framework, then start XAML.
#[cfg(windows)]
pub fn run() -> windows_core::Result<()> {
    boot::run()
}
