use std::sync::Arc;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;

use super::{
    ValidationState, audio::AudioSettingsViewModel, capture::CaptureViewModel,
    editor::SettingsEditor, general::GeneralSettingsViewModel, input::InputSettingsViewModel,
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
        registry: Arc<SystemRegistry>,
        supported_sample_rates: Arc<[u32]>,
    ) -> Self {
        let factories: Vec<Arc<dyn nerust_core_traits::factory::CoreFactory>> =
            registry.all().iter().map(Arc::clone).collect();

        let mut editor = SettingsEditor::new(snapshot, registry, supported_sample_rates);

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
    use nerust_gui_runtime::settings::apply::validate_shared_settings;
    use nerust_gui_shell::{session::input::build_topology, settings::bindings::conflicting_keys};

    let mut issues = Vec::new();

    // Storage policy validation
    if let Err(e) = validate_shared_settings(&state.draft.shared) {
        issues.push(super::ValidationIssue {
            scope: super::ValidationScope::Persistence,
            message: e.to_string(),
        });
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
    for factory in state.registry.all() {
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
