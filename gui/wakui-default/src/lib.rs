//! Flavor selection over the `wakui` framework.
//!
//! Depending on this crate alone runs the platform-default flavor
//! (gtk on Linux, tao elsewhere) with no feature flags and no flavor
//! references downstream. The `gtk` / `tao` features additionally
//! compile the off-default flavor; call [`run_gtk`] / [`run_tao`]
//! explicitly to run it. Custom flavors depend on `wakui` only and
//! never touch this crate.

#[cfg(any(target_os = "linux", feature = "gtk"))]
pub use wakui_gtk::run as run_gtk;
#[cfg(any(not(target_os = "linux"), feature = "tao"))]
pub use wakui_tao::run as run_tao;

/// Platform-default entry: gtk on Linux, tao elsewhere.
#[cfg(target_os = "linux")]
pub fn run(boot: wakui::bootstrap::Boot, rom_path: Option<std::path::PathBuf>) -> i32 {
    run_gtk(boot, rom_path)
}

/// Platform-default entry: gtk on Linux, tao elsewhere.
#[cfg(not(target_os = "linux"))]
pub fn run(boot: wakui::bootstrap::Boot, rom_path: Option<std::path::PathBuf>) -> i32 {
    run_tao(boot, rom_path)
}
