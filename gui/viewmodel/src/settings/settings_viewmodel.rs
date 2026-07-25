use std::{rc::Rc, sync::Arc};

use crate::settings::catalog::FactoryCatalog;
use nerust_gui_settings::snapshot::SettingsSnapshot;

use super::{
    ValidationState, audio::AudioSettingsViewModel, capture::CaptureViewModel,
    editor::{SettingsEditor, StoragePathValidator},
    general::GeneralSettingsViewModel, input::InputSettingsViewModel,
    property::ReadOnlyObservableProperty, system::SystemSettingsViewModel,
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
    pub capture: CaptureViewModel,
    systems: Vec<SystemSettingsViewModel>,
    inputs: Vec<InputSettingsViewModel>,
}

impl SettingsViewModel {
    pub fn new(
        snapshot: SettingsSnapshot,
        factories: Vec<Arc<dyn nerust_core_traits::factory::CoreFactory>>,
        supported_sample_rates: Arc<[u32]>,
        storage_validator: Rc<dyn StoragePathValidator>,
    ) -> Self {
        let catalog = FactoryCatalog::new(factories.clone());

        let mut editor = SettingsEditor::new(
            snapshot,
            catalog,
            supported_sample_rates,
            storage_validator,
        );

        editor.set_validator(validator);

        let revision = editor.revision_prop();

        let general = GeneralSettingsViewModel::new(&editor);
        let video = VideoSettingsViewModel::new(&editor);
        let audio = AudioSettingsViewModel::new(&editor);
        let capture = CaptureViewModel::new(&editor);

        let systems: Vec<SystemSettingsViewModel> = factories
            .iter()
            .map(|f| SystemSettingsViewModel::new(&editor, f))
            .collect();

        let inputs: Vec<InputSettingsViewModel> = factories
            .iter()
            .map(|f| InputSettingsViewModel::new(&editor, f))
            .collect();

        editor.projections().seal();

        Self {
            editor,
            revision,
            general,
            video,
            audio,
            capture,
            systems,
            inputs,
        }
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

fn validator(state: &super::EditorState) -> super::ValidationState {
    use nerust_gui_settings::shared::StoragePolicy;
    use nerust_settings_core::{bindings::conflicting_keys, input::build_topology};

    let mut issues = Vec::new();

    // Storage policy validation
    if matches!(
        state.draft.shared.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ) {
        if let Some(ref path) = state.draft.shared.persistence.storage_directory {
            if let Err(e) = state.storage_validator.validate(path) {
                issues.push(super::ValidationIssue {
                    scope: super::ValidationScope::Persistence,
                    message: e.to_string(),
                });
            }
        } else {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Persistence,
                message: "Custom storage directory required".into(),
            });
        }
    }

    // Audio range validation
    if !(0..=100).contains(&state.draft.local.audio.master_volume_percent) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            message: "Master volume must be between 0 and 100".into(),
        });
    }
    if !(1..=192_000).contains(&state.draft.local.audio.sample_rate) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            message: "Sample rate must be between 1 and 192000".into(),
        });
    }
    if !(10..=200).contains(&state.draft.local.audio.latency_ms) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Audio,
            message: "Audio latency must be between 10 and 200 ms".into(),
        });
    }

    // Per-system validation: controller assignment + key conflicts
    for factory in state.catalog.all() {
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
                    .slots
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
                message: format!(
                    "{}: {} unknown slot ID(s)",
                    factory.display_name(),
                    unknown_slots
                ),
            });
        }
        if unknown_profiles > 0 {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Input(sid.clone_box()),
                message: format!(
                    "{}: {} unknown controller profile ID(s)",
                    factory.display_name(),
                    unknown_profiles
                ),
            });
        }
        let has_controller = assignments.iter().any(|(_, c)| c.is_some());
        if !has_controller {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Input(sid.clone_box()),
                message: format!(
                    "{}: At least one controller must be assigned",
                    factory.display_name()
                ),
            });
        }
        let topology = build_topology(&assignments, input_factory.slots());
        for (key, labels) in
            conflicting_keys(&state.draft.shared, &topology, factory.system_id().as_ref())
        {
            issues.push(super::ValidationIssue {
                scope: super::ValidationScope::Input(sid.clone_box()),
                message: format!("{}: {}", key.label(), labels.join(", ")),
            });
        }
    }

    super::ValidationState { issues }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::editor::SettingsEditor;
    use crate::settings::test_support::{P1_SLOT, TestCoreFactory, TestInputFactory};
    use nerust_core_traits::factory::CoreFactory;

    #[test]
    fn validator_detects_unknown_profile() {
        use nerust_gui_settings::snapshot::SettingsSnapshot;
        use nerust_gui_settings::{
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

        let factory: Arc<dyn nerust_core_traits::factory::CoreFactory> = Arc::new(test_factory);
        let catalog = crate::settings::catalog::FactoryCatalog::new(vec![factory]);
        let supported_sample_rates: Arc<[u32]> = Arc::new([]);

        let mut editor = SettingsEditor::new(
            snapshot,
            catalog,
            supported_sample_rates,
            Rc::new(crate::settings::NoopStoragePathValidator) as Rc<dyn crate::settings::StoragePathValidator>,
        );
        editor.set_validator(validator);

        // finish() should reject because of the unknown profile
        assert!(editor.finish().is_err());

        // Check that the issue mentions unknown profile
        let err = editor.finish().unwrap_err();
        let has_unknown_profile = err
            .issues
            .iter()
            .any(|i| i.message.contains("unknown controller profile"));
        assert!(
            has_unknown_profile,
            "validator should flag unknown profile ID"
        );
    }
}
