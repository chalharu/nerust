//! Build-time guard for the macOS link environment.
//!
//! An ambient `LIBRARY_PATH` silently redirects `-l` resolution at
//! link time. This once shipped a macOS binary that loaded foreign
//! image codecs and crashed inside ImageIO on window resize, with no
//! hint at the true cause. Fail fast with an actionable message
//! instead. Linux targets are deliberately exempt: an ambient
//! `LIBRARY_PATH` can be load-bearing there (custom prefixes, Nix,
//! conda envs).
//!
//! Note: this script runs on the HOST, so the gate keys off
//! `CARGO_CFG_TARGET_OS` (the link target), not `#[cfg(target_os)]`.

fn main() {
    println!("cargo:rerun-if-env-changed=LIBRARY_PATH");
    let targeting_macos = std::env::var("CARGO_CFG_TARGET_OS")
        .map(|target| target == "macos")
        .unwrap_or(false);
    if !targeting_macos {
        return;
    }
    if let Ok(value) = std::env::var("LIBRARY_PATH")
        && !value.is_empty()
    {
        panic!(
            "LIBRARY_PATH is set ({value:?}) but this workspace must not inherit ambient link \
             search paths when targeting macOS: it silently redirects `-l` resolution and has \
             produced binaries that crash inside system frameworks. Unset it for the build, e.g. \
             `LIBRARY_PATH= cargo build --features tao`. If you genuinely need extra search \
             paths, pass them per invocation instead of exporting them."
        );
    }
}
