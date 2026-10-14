//! macOS link guard: scrub ambient `LIBRARY_PATH` from rustc
//! invocations (Linux exempt: it can be load-bearing there).
//! Gate keys off `CARGO_CFG_TARGET_OS`, not `#[cfg(target_os)]`,
//! because this script runs on the HOST.

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
