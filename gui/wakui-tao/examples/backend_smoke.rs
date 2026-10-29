//! Minimal backend sketch: `wakui` plus `wakui_tao` only.
//! Builds a control screen through the backend interpreter, maps native
//! input through the backend, and runs the framework conformance kit.
//! Run with `cargo run -p wakui_tao --example backend_smoke`.

use wakui_core::app::backend::BackendRegistry;
use wakui_core::app::binding::PickerSink;
use wakui_core::scene::control::ControlVisitor;
use wakui_core::support::conformance::{ConformanceBackend, conformance_screen, run_conformance};
use wakui_tao::{input::keyboard, register, scene::render};

struct Printer;

impl ControlVisitor for Printer {
    fn control(&mut self, control: &wakui_core::scene::control::Control) {
        println!("control: {}", control.label);
    }
}

impl PickerSink<String> for Printer {
    fn submit(
        &mut self,
        _target: wakui_core::scene::control::ControlId,
        _request: wakui_core::scene::control::Id,
        _payload: Option<String>,
    ) {
    }
}

fn main() {
    // A backend behind the framework traits: visit, then interpret.
    let screen = conformance_screen();
    let mut printer = Printer;
    screen.visit(&mut printer);
    let element = render::build_screen(&screen);
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
