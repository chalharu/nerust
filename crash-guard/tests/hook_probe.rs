//! Subprocess probe: the hook + `run_guarded` touch process-global
//! state, so firing is verified out-of-process (exit code + stderr).

use std::process::Command;

fn probe_binary() -> std::path::PathBuf {
    let mut dir = std::env::current_exe().expect("test executable path");
    dir.pop();
    if dir.file_name().is_some_and(|name| name == "deps") {
        dir.pop();
    }
    dir.join("examples").join("crash_probe")
}

#[test]
fn guard_exits_101_with_logged_report() {
    let binary = probe_binary();
    assert!(binary.is_file(), "probe not built: {}", binary.display());
    let output = Command::new(&binary)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("probe spawn");
    assert_eq!(output.status.code(), Some(101), "probe must exit(101)");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("probe crash"), "{stderr}");
    assert!(stderr.contains("crashed on thread"), "{stderr}");
}
