//! Backend conformance kit (Phase 4). A third-party backend proves
//! framework compliance by driving [`ConformanceBackend`] through
//! [`run_conformance`]: screen visit order, construction-vs-patch mapper
//! output, picker correlation, registry selection, and one
//! [`drive_turn`](crate::loop_driver::drive_turn) with a pending flight.
//! `tao`/`gtk` stay native frontends and are not driven by this kit; it
//! exists for future `wakui_*` backends and documents the backend contract
//! in executable form.

use crate::{
    backend::BackendRegistry,
    binding::PickerSink,
    control::{
        Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlId, ControlMetadata,
        ControlScreen, ControlVisitor, Id, Node, Page,
    },
    leaf::{ActionLeaf, ControlLeaf, ToggleLeaf},
    loop_driver::{LoopHost, TurnLiveness, WakeNeed, drive_turn, flight_backstop},
    mapper::map_updates,
};

fn control(label: &str, leaf: impl ControlLeaf) -> Control {
    Control {
        id: Id::fresh(),
        label: label.to_string(),
        metadata: ControlMetadata::new(label),
        display: ControlDisplay::Standard,
        leaf: Box::new(leaf),
    }
}

fn layout(children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new("conformance");
    metadata.hints.spacing = Some(12.0);
    metadata.hints.indent = Some(0.0);
    Container {
        id: Id::fresh(),
        label: String::new(),
        metadata,
        header: None,
        policy: Box::new(ColumnPolicy),
        children,
    }
}

/// Minimal two-control screen: one toggle in a page, one action in the footer.
pub fn conformance_screen() -> ControlScreen {
    let toggle = control("Mute", ToggleLeaf { value: false });
    let page = Page {
        id: Id::fresh(),
        label: "Audio".to_string(),
        selected: true,
        select: control("Audio", ActionLeaf),
        body: layout(vec![Node::Control(toggle)]),
    };
    ControlScreen {
        title: "Conformance".to_string(),
        pages: vec![page],
        footer: layout(vec![Node::Control(control("OK", ActionLeaf))]),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

/// Framework-side fake backend. Real backends mirror this shape: record
/// visits, execute mapper updates, correlate picker payloads by request id.
#[derive(Debug)]
pub struct ConformanceBackend {
    /// Control labels in visit order.
    pub visited: Vec<String>,
    /// Full construction updates applied (`old == None`).
    pub constructed: usize,
    /// Patch updates applied (`old == Some`).
    pub patched: usize,
    /// Picker payloads by `(target, request)` correlation.
    pub picker: Vec<(ControlId, Id, Option<String>)>,
    /// Redraw requests issued by turn driving.
    pub redraws: u32,
    /// Pending flight injected for the driven turn.
    pub flight_pending: bool,
    /// Liveness returned by the driven turn (running core by default).
    pub live: TurnLiveness,
}

impl Default for ConformanceBackend {
    fn default() -> Self {
        Self {
            visited: Vec::new(),
            constructed: 0,
            patched: 0,
            picker: Vec::new(),
            redraws: 0,
            flight_pending: false,
            live: TurnLiveness {
                loaded: true,
                paused: false,
                main_active: false,
                live_follow: false,
            },
        }
    }
}

impl ControlVisitor for ConformanceBackend {
    fn control(&mut self, control: &Control) {
        self.visited.push(control.label.clone());
        // Surface nested controls exactly once, like flavors do.
        let ctx = crate::leaf::LeafCtx {
            id: control.id,
            hints: control.metadata.hints,
        };
        control.leaf.walk(&ctx, self);
    }
}

impl PickerSink<String> for ConformanceBackend {
    fn submit(&mut self, target: ControlId, request: Id, payload: Option<String>) {
        self.picker.push((target, request, payload));
    }
}

impl LoopHost for ConformanceBackend {
    type Frame = ();

    fn before_turn(&mut self) {}
    fn frame_ctx(&mut self) {}
    fn chrome(&mut self, _ctx: &Self::Frame) {}
    fn pump_widgets(&mut self) -> WakeNeed {
        WakeNeed::default()
    }
    fn pump_dialog(&mut self, _ctx: &Self::Frame) -> WakeNeed {
        WakeNeed::default()
    }
    fn pump_aux(&mut self) -> WakeNeed {
        WakeNeed {
            flight_pending: self.flight_pending,
            ..WakeNeed::default()
        }
    }
    fn late_liveness(&mut self) -> TurnLiveness {
        self.live
    }
    fn request_redraw(&mut self) {
        self.redraws += 1;
    }
}

/// Executable backend contract. Panics (via `assert!`) on any violation so
/// third-party backends get a single entry point to run.
pub fn run_conformance(backend: &mut ConformanceBackend) {
    // 1. Visit order: page body first, footer last.
    let screen = conformance_screen();
    screen.visit(backend);
    assert_eq!(
        backend.visited,
        ["Mute", "OK"],
        "visit order is page body then footer"
    );

    // 2. Construction applies full updates; an unchanged patch applies none.
    let toggle = match &screen.pages[0].body.children[0] {
        Node::Control(control) => control.clone(),
        _ => panic!("conformance screen keeps its toggle"),
    };
    let built = map_updates(None, &toggle);
    assert!(!built.is_empty(), "construction emits updates");
    backend.constructed += built.len();
    let no_op = map_updates(Some(&toggle), &toggle);
    assert!(no_op.is_empty(), "unchanged controls patch to nothing");
    let mut flipped = toggle.clone();
    flipped.leaf = Box::new(ToggleLeaf { value: true });
    let patched = map_updates(Some(&toggle), &flipped);
    assert!(!patched.is_empty(), "a value flip patches");
    backend.patched += patched.len();

    // 3. Picker correlation survives the round trip.
    let request = Id::fresh();
    backend.submit(toggle.id, request, Some("/tmp/rom".to_string()));
    assert_eq!(backend.picker.len(), 1);
    assert_eq!(backend.picker[0].1, request);

    // 4. Registry selection prefers the explicit backend.
    let mut registry = BackendRegistry::default();
    registry.register("conformance");
    registry.register("other");
    assert_eq!(registry.select(Some("other")), Some("other"));
    assert_eq!(registry.select(None), Some("conformance"));

    // 5. A pending flight forces a backstop deadline and a redraw.
    // The fake backend reports a running core, so the turn redraws.
    backend.flight_pending = true;
    let before = std::time::Instant::now();
    let outcome = drive_turn(backend);
    assert_eq!(backend.redraws, 1, "a running-core turn redraws");
    let deadline = outcome.deadline.expect("pending flight sets a deadline");
    assert!(
        deadline >= flight_backstop(before),
        "deadline honors the 30ms flight backstop"
    );
    let _ = ColumnPolicy;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformance_kit_passes_on_the_fake_backend() {
        let mut backend = ConformanceBackend::default();
        run_conformance(&mut backend);
        assert_eq!(backend.visited, ["Mute", "OK"]);
        assert!(backend.constructed > 0);
        assert!(backend.patched > 0);
        assert_eq!(backend.redraws, 1);
    }
}
