#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub use wakui_gtk::*;

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use wakui_tao::*;
