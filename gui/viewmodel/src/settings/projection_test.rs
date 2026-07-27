use std::rc::Rc;
use std::sync::Arc;

use nerust_gui_settings::snapshot::SettingsSnapshot;
use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

use crate::settings::{
    EditorState, ValidationState,
    catalog::FactoryCatalog,
    editor::{NoopStoragePathValidator, SettingsEditor, StoragePathValidator},
    projection::{PreparedProjection, ProjectionHub, ProjectionNode},
};

fn test_editor() -> SettingsEditor {
    let snapshot = SettingsSnapshot {
        shared: DesktopSharedSettings::default(),
        local: HostBackendLocalSettings::default(),
        app_state: DesktopAppState::default(),
    };
    SettingsEditor::new(
        snapshot,
        FactoryCatalog::new(vec![]).unwrap(),
        Arc::new([]),
        Rc::new(NoopStoragePathValidator) as Rc<dyn StoragePathValidator>,
        |_| ValidationState { issues: vec![] },
    )
}

#[test]
fn register_adds_node() {
    let hub = ProjectionHub::new();
    assert_eq!(hub.node_count(), 0);
    hub.register("test", 0usize, |_| 1);
    assert_eq!(hub.node_count(), 1);
}

#[test]
fn prepare_all_returns_prepared_projections_for_changed_values() {
    let hub = ProjectionHub::new();
    let _prop = hub.register("muted", false, |state| state.draft.local.audio.muted);

    let editor = test_editor();
    let candidate = editor.current();
    let _prepared = hub.prepare_all(&candidate);
    drop(candidate);

    editor
        .transact(|state| {
            state.draft_mut().local.audio.muted = true;
            Ok(())
        })
        .unwrap();
    let updated = editor.current();
    let prepared2 = hub.prepare_all(&updated);
    assert_eq!(prepared2.len(), 1);
}

#[test]
fn seal_prevents_registration() {
    let hub = ProjectionHub::new();
    hub.seal();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hub.register("after_seal", 0usize, |_| 0);
    }));
    assert!(result.is_err(), "register after seal should panic");
}

#[test]
fn prepare_all_skips_unchanged_projections() {
    let hub = ProjectionHub::new();
    hub.register("constant", 42usize, |_| 42);

    let editor = test_editor();
    let candidate = editor.current();
    let prepared = hub.prepare_all(&candidate);
    assert!(prepared.is_empty());
}

// --- register_node tests: verify custom ProjectionNode impls ---

struct CountingProjection {
    name: &'static str,
    prepare_count: std::cell::Cell<usize>,
}

impl CountingProjection {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            prepare_count: std::cell::Cell::new(0),
        }
    }
}

impl ProjectionNode for CountingProjection {
    fn name(&self) -> &'static str {
        self.name
    }

    fn prepare(&self, _candidate: &EditorState) -> Option<Box<dyn PreparedProjection>> {
        self.prepare_count.set(self.prepare_count.get() + 1);
        None
    }

    fn is_synced(&self, _current: &EditorState) -> bool {
        true
    }
}

#[test]
fn register_node_adds_custom_projector() {
    let hub = ProjectionHub::new();
    let node = Rc::new(CountingProjection::new("custom"));
    hub.register_node(Rc::clone(&node) as Rc<dyn ProjectionNode>);
    assert_eq!(hub.node_count(), 1);

    let editor = test_editor();
    let _prepared = hub.prepare_all(&editor.current());
    assert_eq!(node.prepare_count.get(), 1, "prepare should be called");
}

#[test]
fn register_node_is_invoked_after_seal() {
    let hub = ProjectionHub::new();
    let node = Rc::new(CountingProjection::new("sealed-test"));
    hub.register_node(Rc::clone(&node) as Rc<dyn ProjectionNode>);
    hub.seal();

    let editor = test_editor();
    let _prepared = hub.prepare_all(&editor.current());
    assert_eq!(node.prepare_count.get(), 1);
}

#[test]
fn register_node_panics_on_sealed_hub() {
    let hub = ProjectionHub::new();
    hub.seal();
    let node = Rc::new(CountingProjection::new("post-seal"));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hub.register_node(node as Rc<dyn ProjectionNode>);
    }));
    assert!(result.is_err(), "register_node after seal should panic");
}
