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

        editor.set_validator(|_state| super::ValidationState { issues: vec![] });

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
