//! Minimal backend sketch: `wakui` plus `wakui_gtk` only.
//! Registers the flavor backend and runs the framework conformance kit.
//! Flavor-internal machinery (rendering, input mapping, widget construction)
//! is crate-internal and covered by the flavor's own display-gated tests,
//! not by third-party code.
//! Run with `cargo run -p wakui_gtk --example backend_smoke`.

use wakui_core::app::backend::BackendRegistry;
use wakui_core::support::conformance::{ConformanceBackend, run_conformance};
use wakui_gtk::register;

fn main() {
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
