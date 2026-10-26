//! macOS demo shell: real AppKit frontend over the shared host.
//!
//! Build with `cargo check --target x86_64-apple-darwin -p nerust_mac
//! --example macos_demo` (type-checks against AppKit bindings; running
//! needs macOS). On other platforms this example is a stub.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod dbg;
#[cfg(target_os = "macos")]
mod ppu;
#[cfg(target_os = "macos")]
mod settings;

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macos_demo requires macOS (type-check with --target x86_64-apple-darwin)");
}
