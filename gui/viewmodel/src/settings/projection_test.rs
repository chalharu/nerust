use std::rc::Rc;
use std::sync::Arc;

use nerust_gui_settings::snapshot::SettingsSnapshot;
use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

use crate::settings::{
    ValidationState,
    catalog::FactoryCatalog,
    editor::{NoopStoragePathValidator, SettingsEditor, StoragePathValidator},
    projection::ProjectionHub,
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
