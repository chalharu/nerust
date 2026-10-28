//! Minimal backend sketch: `wakui` plus `wakui_gtk` only.
//! Maps native input through the backend and runs the framework
//! conformance kit. Widget construction needs a display, so it stays in
//! the display-gated backend test, not here.
//! Run with `cargo run -p wakui-gtk --example backend_smoke`.

use wakui::backend::BackendRegistry;
use wakui::conformance::{ConformanceBackend, run_conformance};
use wakui_gtk::{keyboard, register};

fn main() {
    // Native input maps without the app crates.
    assert_eq!(keyboard::key_from_gdk(gdk::Key::F6), Some(wakui::Key::F6));
    assert_eq!(keyboard::key_from_gdk(gdk::Key::a), Some(wakui::Key::KeyA));

    // Registry selection prefers the explicit backend.
    let mut registry = BackendRegistry::default();
    register(&mut registry);
    assert_eq!(registry.select(Some("gtk")), Some("gtk"));

    // The full contract, driven on the framework fake backend.
    let mut backend = ConformanceBackend::default();
    run_conformance(&mut backend);
    println!(
        "conformance ok: {} visited, {} constructed, {} patched, {} redraws",
        backend.visited.len(),
        backend.constructed,
        backend.patched,
        backend.redraws
    );
}
