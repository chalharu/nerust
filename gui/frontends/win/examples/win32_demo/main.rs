//! Win32 demo shell: real HWND frontend over the shared host.
//!
//! Build with `cargo build --target x86_64-pc-windows-gnu -p nerust_win
//! --example win32_demo` and run the exe under Wine. On other platforms
//! this example is a stub.

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod dbg;
#[cfg(windows)]
mod fluent;
#[cfg(windows)]
mod ppu;
#[cfg(windows)]
mod settings;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("win32_demo requires Windows (run the cross-compiled exe under Wine)");
}
