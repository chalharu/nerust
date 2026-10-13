//! Neutralize ambient `LIBRARY_PATH` for the macOS link.
//!
//! An ambient `LIBRARY_PATH` silently redirects `-l` resolution at
//! link time. This once shipped a macOS binary that loaded foreign
//! image codecs and crashed inside ImageIO on window resize, with no
//! hint at the true cause. Rather than failing the build, scrub the
//! variable from the rustc invocations compiling this package: the
//! final link of the `nerust` binary runs in that environment, so
//! foreign search paths can no longer leak into it. Linux targets are
//! deliberately exempt: an ambient `LIBRARY_PATH` can be load-bearing
//! there (custom prefixes, Nix, conda envs).
//!
//! Note: this script runs on the HOST, so the gate keys off
//! `CARGO_CFG_TARGET_OS` (the link target), not `#[cfg(target_os)]`.

fn main() {
    println!("cargo:rerun-if-env-changed=LIBRARY_PATH");

    #[cfg(not(feature = "gtk"))]
    {
        let targeting_macos =
            std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|target| target == "macos");
        if targeting_macos && std::env::var("LIBRARY_PATH").is_ok_and(|value| !value.is_empty()) {
            println!("cargo:rustc-env=LIBRARY_PATH=");
        }
    }
}
