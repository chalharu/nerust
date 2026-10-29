//! Minimal backend sketch: `wakui` plus `wakui_tao` only.
//! Builds a control screen through the backend interpreter, maps native
//! input through the backend, and runs the framework conformance kit.
//! Run with `cargo run -p wakui-tao --example backend_smoke`.

use wakui_core::backend::BackendRegistry;
use wakui_core::binding::PickerSink;
use wakui_core::conformance::{ConformanceBackend, conformance_screen, run_conformance};
use wakui_core::control::ControlVisitor;
use wakui_tao::{controls, keyboard, register};

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
    // A backend behind the framework traits: visit, then interpret.
    let screen = conformance_screen();
    let mut printer = Printer;
    screen.visit(&mut printer);
    let element = controls::build_screen(&screen);
    drop(element);

    // Native input maps without the app crates.
    let physical = iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Enter);
    assert_eq!(
        keyboard::key_from_physical(physical),
        Some(wakui_core::Key::Enter)
    );

    // Registry selection prefers the explicit backend.
    let mut registry = BackendRegistry::default();
    register(&mut registry);
    assert_eq!(registry.select(Some("tao")), Some("tao"));

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
