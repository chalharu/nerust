use std::{collections::HashMap, rc::Rc, sync::Arc};

use crate::settings::catalog::FactoryCatalog;
use crate::settings::topology_adapter::build_topology;
use nerust_input_traits::{InputAssignments, InputPorts};
use nerust_settings_core::bindings::{BindingIndex, conflicting_keys_indexed};
use nerust_settings_core::model::snapshot::SettingsSnapshot;
use nerust_system_traits::factory::CoreFactory;

use super::{
    ValidationState,
    audio::AudioSettingsViewModel,
    capture::CaptureViewModel,
    editor::{SettingsEditor, StoragePathValidator},
    gamepad::GamepadViewModel,
    general::GeneralSettingsViewModel,
    input::InputSettingsViewModel,
    notification::NotificationSettingsViewModel,
    property::ReadOnlyObservableProperty,
    shortcuts::ShortcutsViewModel,
    system::SystemSettingsViewModel,
    video::VideoSettingsViewModel,
};

/// Root settings view model, composing all page-level sub-view models.
#[derive(Clone)]
pub struct SettingsViewModel {
    editor: SettingsEditor,
    pub revision: ReadOnlyObservableProperty<u64>,
    pub general: GeneralSettingsViewModel,
    pub video: VideoSettingsViewModel,
    pub audio: AudioSettingsViewModel,
    pub notifications: NotificationSettingsViewModel,
    pub capture: CaptureViewModel,
    pub shortcuts: ShortcutsViewModel,
    pub gamepads: GamepadViewModel,
    systems: Vec<SystemSettingsViewModel>,
    inputs: Vec<InputSettingsViewModel>,
}

impl SettingsViewModel {
    pub fn new(
        snapshot: SettingsSnapshot,
        factories: Vec<Arc<dyn nerust_system_traits::factory::CoreFactory>>,
        supported_sample_rates: Arc<[u32]>,
        storage_validator: Rc<dyn StoragePathValidator>,
    ) -> Result<Self, super::catalog::CatalogError> {
        let catalog = FactoryCatalog::new(factories.clone())?;

        let editor = SettingsEditor::new(
            snapshot,
            catalog,
            supported_sample_rates,
            storage_validator,
            validator,
        );

        let revision = editor.revision_prop();

        // Create sub-view models (register projectors with the hub)
        let general = GeneralSettingsViewModel::new(&editor);
        let video = VideoSettingsViewModel::new(&editor);
        let audio = AudioSettingsViewModel::new(&editor);
        let notifications = NotificationSettingsViewModel::new(&editor);
        let capture = CaptureViewModel::new(&editor);
        let shortcuts = ShortcutsViewModel::new(&editor);
        let gamepads = GamepadViewModel::new(&editor);
        let systems: Vec<SystemSettingsViewModel> = factories
            .iter()
            .map(|f| SystemSettingsViewModel::new(&editor, f))
            .collect();
        let inputs: Vec<InputSettingsViewModel> = factories
            .iter()
            .map(|f| InputSettingsViewModel::new(&editor, f))
            .collect();

        editor.projections().seal();

        Ok(Self {
            revision,
            general,
            video,
            audio,
            notifications,
            capture,
            shortcuts,
            gamepads,
            systems,
            inputs,
            editor,
        })
    }

    pub fn snapshot(&self) -> SettingsSnapshot {
        self.editor.snapshot()
    }

    pub fn finish(&self) -> Result<SettingsSnapshot, ValidationState> {
        self.editor.finish()
    }

    pub fn systems(&self) -> &[SystemSettingsViewModel] {
        &self.systems
    }

    pub fn inputs(&self) -> &[InputSettingsViewModel] {
        &self.inputs
    }
}

fn validate_storage_policy(state: &super::EditorState, issues: &mut Vec<super::ValidationIssue>) {
    match nerust_gui_runtime::settings::apply::custom_storage_path(&state.draft.shared) {
        Ok(Some(path)) => {
            if let Err(e) = state.storage_validator.validate(path) {
                issues.push(super::ValidationIssue {
                    scope: super::ValidationScope::Persistence,
                    kind: super::ValidationIssueKind::Storage(e.to_string()),
                });
            }
        }
        Err(error) => {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Persistence,
                kind: super::ValidationIssueKind::Storage(error.to_string()),
            });
        }
        Ok(None) => {}
    }
}

fn validate_audio_settings(state: &super::EditorState, issues: &mut Vec<super::ValidationIssue>) {
    if !(0..=100).contains(&state.draft.local.audio.master_volume_percent) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            kind: super::ValidationIssueKind::VolumeOutOfRange,
        });
    }
    if !(1..=192_000).contains(&state.draft.local.audio.sample_rate) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            kind: super::ValidationIssueKind::SampleRateOutOfRange,
        });
    }
    if !(10..=200).contains(&state.draft.local.audio.latency_ms) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            kind: super::ValidationIssueKind::LatencyOutOfRange,
        });
    }
}

fn validate_notification_settings(
    state: &super::EditorState,
    issues: &mut Vec<super::ValidationIssue>,
) {
    // Durations guard against 0ms (invisible) and absurd holds.
    for (value, kind) in [
        (
            state.draft.local.notifications.info_duration_ms,
            super::ValidationIssueKind::InfoDurationOutOfRange,
        ),
        (
            state.draft.local.notifications.warning_duration_ms,
            super::ValidationIssueKind::WarningDurationOutOfRange,
        ),
    ] {
        if !(1_000..=30_000).contains(&value) {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Notifications,
                kind,
            });
        }
    }
}

fn validate_factory_settings(
    state: &super::EditorState,
    factory: &Arc<dyn CoreFactory>,
    issues: &mut Vec<super::ValidationIssue>,
) {
    let sid = factory.system_id();
    let input_factory = factory.input_system_factory();
    let pairs: Vec<(String, Option<String>)> = state
        .draft
        .app_state
        .controller_assignments
        .get(&sid)
        .cloned()
        .unwrap_or_else(|| {
            input_factory
                .default_assignments()
                .slots()
                .iter()
                .map(|(slot_id, ctrl)| {
                    (
                        slot_id.to_string(),
                        ctrl.as_ref().map(|p| p.profile_id().to_string()),
                    )
                })
                .collect()
        });
    let mut unknown_slots = 0u32;
    let mut unknown_profiles = 0u32;
    let assignments: Vec<_> = pairs
        .iter()
        .filter_map(|(slot_id, ctrl_opt)| {
            let att = match input_factory.resolve_slot(slot_id) {
                Some(a) => a,
                None => {
                    unknown_slots += 1;
                    return None;
                }
            };
            let profile = match ctrl_opt.as_ref() {
                Some(id) => match input_factory.resolve_controller(id) {
                    Some(p) => Some(p),
                    None => {
                        unknown_profiles += 1;
                        None
                    }
                },
                None => None,
            };
            Some((att, profile))
        })
        .collect();
    if unknown_slots > 0 {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Input(sid.clone_box()),
            kind: super::ValidationIssueKind::UnknownSlots {
                system: factory.display_name().to_string(),
                count: unknown_slots,
            },
        });
    }
    if unknown_profiles > 0 {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Input(sid.clone_box()),
            kind: super::ValidationIssueKind::UnknownProfiles {
                system: factory.display_name().to_string(),
                count: unknown_profiles,
            },
        });
    }
    let has_controller = assignments.iter().any(|(_, c)| c.is_some());
    if !has_controller {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Input(sid.clone_box()),
            kind: super::ValidationIssueKind::NoController {
                system: factory.display_name().to_string(),
            },
        });
    }
    // Combinations the UI must never draft (e.g. NES P1 famicom set plus
    // a P2 pad) fail here too, so the rule holds for every frontend and
    // for hand-edited files — not just for screens that disable the
    // conflicting choices.
    if let Err(error) =
        InputAssignments::new(assignments.clone()).placement_plan(input_factory as &dyn InputPorts)
    {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Input(sid.clone_box()),
            kind: super::ValidationIssueKind::Placement {
                system: factory.display_name().to_string(),
                detail: error.to_string(),
            },
        });
    }
    let topology = build_topology(&assignments, input_factory.slots());
    let topology = Rc::new(topology);
    state
        .topology_cache
        .borrow_mut()
        .insert(sid.clone_box(), Rc::clone(&topology));
    // Profile index built once per validation pass, probed per descriptor.
    let index = state
        .draft
        .shared
        .input
        .systems
        .get(factory.system_id().as_ref())
        .and_then(|s| s.implicit_keyboard_profile())
        .map(BindingIndex::for_profile)
        .unwrap_or_default();
    let conflicts = conflicting_keys_indexed(
        &state.draft.shared,
        &topology,
        factory.system_id().as_ref(),
        &index,
    );
    {
        let mut cache = state.conflicts_cache.borrow_mut();
        let cache = cache.get_or_insert_with(HashMap::new);
        cache.insert(factory.system_id(), conflicts.clone());
    }
    for (key, labels) in &conflicts {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Input(sid.clone_box()),
            kind: super::ValidationIssueKind::KeyConflict {
                key: key.label().to_string(),
                labels: labels.join(", "),
            },
        });
    }
}

fn validator(state: &super::EditorState) -> super::ValidationState {
    let mut issues = Vec::new();
    validate_storage_policy(state, &mut issues);
    validate_audio_settings(state, &mut issues);
    validate_notification_settings(state, &mut issues);
    validate_pad_shortcuts(state, &mut issues);
    validate_port_devices(state, &mut issues);
    for factory in state.catalog.all() {
        validate_factory_settings(state, factory, &mut issues);
    }
    super::ValidationState { issues }
}

/// Global pad-shortcut conflicts: one button driving two or more actions.
/// Labels come from the shortcut descriptors, so the message names the
/// rows the user must disambiguate. Same blocking semantics as keyboard
/// key conflicts.
fn validate_pad_shortcuts(state: &super::EditorState, issues: &mut Vec<super::ValidationIssue>) {
    use std::collections::HashMap;
    let labels: HashMap<_, _> = nerust_settings_core::bindings::descriptors::shortcut_descriptors()
        .iter()
        .map(|desc| (desc.action, desc.label))
        .collect();
    for (button, actions) in nerust_settings_core::model::input::pad_shortcut_conflicts(
        &state.draft.shared.input.shortcuts.keyboard,
    ) {
        let names: Vec<&str> = actions
            .iter()
            .map(|action| labels.get(action).copied().unwrap_or("?"))
            .collect();
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Controllers,
            kind: super::ValidationIssueKind::PadConflict {
                button: button.label().to_string(),
                labels: names.join(", "),
            },
        });
    }
}

/// Global port device sets: exclusive members (gamepads) must not sit in
/// two ports at once. Keyboards share freely, so they never issue here.
fn validate_port_devices(state: &super::EditorState, issues: &mut Vec<super::ValidationIssue>) {
    for (member, ports) in
        nerust_settings_core::input::exclusive_member_conflicts(&state.draft.app_state.port_devices)
    {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Controllers,
            kind: super::ValidationIssueKind::ExclusiveMember {
                member,
                ports: ports.join(", "),
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::editor::SettingsEditor;
    use crate::settings::test_support::{P1_SLOT, TestCoreFactory, TestInputFactory};
    use nerust_system_traits::factory::CoreFactory;

    #[test]
    fn saf_custom_storage_uses_shared_selection_validation() {
        use nerust_settings_core::model::shared::StoragePolicy;
        let mut snapshot = SettingsSnapshot {
            shared: Default::default(),
            local: Default::default(),
            app_state: Default::default(),
        };
        snapshot.shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        snapshot.shared.persistence.storage_document_tree_uri =
            Some("content://provider/tree/saves".into());
        let make = |snapshot| {
            SettingsViewModel::new(
                snapshot,
                vec![],
                Arc::new([]),
                Rc::new(crate::settings::NoopStoragePathValidator),
            )
            .unwrap()
        };
        for policy in [StoragePolicy::Sidecar, StoragePolicy::CustomDirectory] {
            snapshot.shared.persistence.storage_policy = policy;
            for uri in [
                "content://provider/tree/saves",
                "",
                "content:///tree/saves",
                "file:///tmp/saves",
                "content://provider/ bad",
                "content://provider/\0",
            ] {
                snapshot.shared.persistence.storage_document_tree_uri = Some(uri.into());
                let runtime_valid =
                    nerust_gui_runtime::settings::apply::validate_shared_settings(&snapshot.shared)
                        .is_ok();
                assert_eq!(
                    make(snapshot.clone()).finish().is_ok(),
                    runtime_valid,
                    "{policy:?}: {uri:?}"
                );
                assert_eq!(runtime_valid, uri == "content://provider/tree/saves");
            }
        }
        snapshot.shared.persistence.storage_document_tree_uri = None;
        assert!(make(snapshot).finish().is_err());
    }

    #[test]
    fn validator_detects_unknown_profile() {
        use nerust_settings_core::model::snapshot::SettingsSnapshot;
        use nerust_settings_core::model::{
            app_state::DesktopAppState, local::HostBackendLocalSettings,
            shared::DesktopSharedSettings,
        };

        let mut snapshot = SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };

        // Insert using the factory's system_id
        let test_factory = TestCoreFactory(TestInputFactory::new());
        let sid = test_factory.system_id();
        snapshot.app_state.controller_assignments.insert(
            sid,
            vec![(P1_SLOT.to_string(), Some("unknown.profile.id".into()))],
        );

        let factory: Arc<dyn nerust_system_traits::factory::CoreFactory> = Arc::new(test_factory);
        let catalog = crate::settings::catalog::FactoryCatalog::new(vec![factory]).unwrap();
        let supported_sample_rates: Arc<[u32]> = Arc::new([]);

        let editor = SettingsEditor::new(
            snapshot,
            catalog,
            supported_sample_rates,
            Rc::new(crate::settings::NoopStoragePathValidator)
                as Rc<dyn crate::settings::StoragePathValidator>,
            validator,
        );

        // finish() should reject because of the unknown profile
        assert!(editor.finish().is_err());

        // Check that the issue mentions unknown profile
        let err = editor.finish().unwrap_err();
        let has_unknown_profile = err.issues.iter().any(|i| {
            matches!(
                i.kind,
                crate::settings::ValidationIssueKind::UnknownProfiles { .. }
            )
        });
        assert!(
            has_unknown_profile,
            "validator should flag unknown profile ID"
        );
    }

    #[test]
    fn validator_flags_shared_pad_shortcuts() {
        use std::sync::Arc;

        use nerust_input_traits::StandardButton;
        use nerust_settings_core::model::{
            app_state::DesktopAppState,
            input::{ShortcutAction, default_shortcut_bindings},
            local::HostBackendLocalSettings,
            shared::DesktopSharedSettings,
            snapshot::SettingsSnapshot,
        };

        use crate::settings::test_support::{TestCoreFactory, TestInputFactory};

        let mut shared = DesktopSharedSettings::default();
        let mut bindings = default_shortcut_bindings();
        bindings
            .iter_mut()
            .find(|binding| binding.action == ShortcutAction::Reset)
            .expect("reset row")
            .pad = Some(StandardButton::Guide);
        shared.input.shortcuts.keyboard = bindings;
        let snapshot = SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        let factory: Arc<dyn nerust_system_traits::factory::CoreFactory> =
            Arc::new(TestCoreFactory(TestInputFactory::new()));
        let vm = crate::settings::SettingsViewModel::new(
            snapshot,
            vec![factory],
            Arc::new([]),
            std::rc::Rc::new(crate::settings::NoopStoragePathValidator)
                as std::rc::Rc<dyn crate::settings::editor::StoragePathValidator>,
        )
        .unwrap();
        let err = vm.finish().expect_err("shared Guide must block submit");
        assert!(
            err.issues.iter().any(|issue| matches!(
                &issue.kind,
                crate::settings::ValidationIssueKind::PadConflict { button, .. }
                if button == "Guide"
            )),
            "shared Guide binding must block with a pad conflict, got {:?}",
            err.issues
        );
    }
}
