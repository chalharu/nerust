use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/AppKit.swift");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "macos" {
        return;
    }
    assert_eq!(
        env::var("HOST").unwrap(),
        env::var("TARGET").unwrap(),
        "AppKit bridge requires a native build"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let library = output.join("libWakui3AppKit.dylib");
    let arch = if env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "aarch64" {
        "arm64"
    } else {
        "x86_64"
    };
    let minimum = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let result = Command::new("xcrun")
        .args([
            "swiftc",
            "-swift-version",
            "5",
            "-O",
            "-emit-library",
            "-module-name",
            "Wakui3AppKit",
            "-target",
        ])
        .arg(format!("{arch}-apple-macosx{minimum}"))
        .args(["-Xlinker", "-install_name", "-Xlinker"])
        .arg(&library)
        .args(["native/AppKit.swift", "-o"])
        .arg(&library)
        .status()
        .expect("Xcode Command Line Tools with Swift are required");
    assert!(result.success(), "AppKit bridge compilation failed");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=dylib=Wakui3AppKit");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", output.display());
}
