//! WinUI application entry point: bootstrap the Windows App Runtime,
//! then start the XAML application. Unpackaged deployment requires the
//! bootstrap; it resolves the 1.4+ framework (or self-contained files
//! beside the exe). Needs real Windows — under Wine the bootstrap
//! aborts (verified: unimplemented `combase!RoOriginateErrorW`).

#[cfg(windows)]
fn main() {
    if let Err(detail) = nerust_winui::run() {
        eprintln!("nerust_winui: {detail}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("nerust_winui requires Windows (WinUI 3 unpackaged app)");
}
