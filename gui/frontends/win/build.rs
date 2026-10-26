//! Link the comctl32-v6 / UTF-8 / PerMonitorV2 manifest into the demo
//! example exes when targeting mingw-w64 (rust-lld cannot embed manifests,
//! so windres compiles the .rc to an object passed to the linker).
//!
//! No-op on every other target; the manifest only affects the examples,
//! never the `nerust_win` library.

fn main() {
    println!("cargo:rerun-if-changed=win32_demo.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.ends_with("-windows-gnu") {
        return;
    }
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest = dir.join("win32_demo.manifest");
    let rc = out.join("win32_demo.rc");
    let rc_text = format!(
        "1 RT_MANIFEST \"{}\"\n",
        manifest.display().to_string().replace('\\', "/")
    );
    if std::fs::write(&rc, rc_text).is_err() {
        println!("cargo:warning=win32_demo manifest rc unwritable");
        return;
    }
    let obj = out.join("win32_demo_res.o");
    let windres =
        std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".to_string());
    match std::process::Command::new(&windres)
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status()
    {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg={}", obj.display());
        }
        _ => println!("cargo:warning={windres} unavailable; building without app manifest"),
    }
}
