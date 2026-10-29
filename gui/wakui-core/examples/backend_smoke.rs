//! Minimal backend sketch: depend on `wakui` only, render a control
//! screen through a recording visitor, patch through the mapper, and drive
//! one turn. Run with `cargo run -p wakui --example backend_smoke`.

use wakui_core::{
    backend::BackendRegistry,
    binding::PickerSink,
    conformance::{ConformanceBackend, conformance_screen, run_conformance},
    control::ControlVisitor,
    mapper::map_updates,
};

struct Printer;

impl ControlVisitor for Printer {
    fn control(&mut self, control: &wakui_core::control::Control) {
        println!("control: {}", control.label);
    }
}

impl PickerSink<String> for Printer {
    fn submit(
        &mut self,
        _target: wakui_core::control::ControlId,
        _request: wakui_core::control::Id,
        _payload: Option<String>,
    ) {
    }
}

fn main() {
    // A backend behind the framework traits: visit, then patch.
    let screen = conformance_screen();
    let mut printer = Printer;
    screen.visit(&mut printer);
    let first = match &screen.pages[0].body.children[0] {
        wakui_core::control::Node::Control(control) => control.clone(),
        _ => panic!("conformance screen keeps its toggle"),
    };
    let updates = map_updates(None, &first);
    println!("construction updates: {}", updates.len());

    // Registry selection prefers the explicit backend.
    let mut registry = BackendRegistry::default();
    registry.register("smoke");
    assert_eq!(registry.select(Some("smoke")), Some("smoke"));

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
