use std::sync::Arc;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;

use super::{
    ValidationState, audio::AudioSettingsViewModel, capture::CaptureViewModel,
    editor::SettingsEditor, general::GeneralSettingsViewModel,
    property::ReadOnlyObservableProperty, video::VideoSettingsViewModel,
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
}

impl SettingsViewModel {
    pub fn new(
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        supported_sample_rates: Arc<[u32]>,
    ) -> Self {
        let mut editor = SettingsEditor::new(snapshot, registry, supported_sample_rates);

        // Set up validator
        editor.set_validator(|_state| super::ValidationState { issues: vec![] });

        let revision = editor.revision_prop();

        // Create sub-view models (they register their projectors with the hub)
        let general = GeneralSettingsViewModel::new(&editor);
        let video = VideoSettingsViewModel::new(&editor);
        let audio = AudioSettingsViewModel::new(&editor);
        let capture = CaptureViewModel::new(&editor);

        // Seal the hub so no more projections can be registered
        editor.projections().seal();

        Self {
            editor,
            revision,
            general,
            video,
            audio,
            capture,
        }
    }

    pub fn snapshot(&self) -> SettingsSnapshot {
        self.editor.snapshot()
    }

    pub fn finish(&self) -> Result<SettingsSnapshot, ValidationState> {
        self.editor.finish()
    }

    pub fn systems(&self) -> &[super::system::SystemSettingsViewModel] {
        &[]
    }

    pub fn inputs(&self) -> &[super::input::InputSettingsViewModel] {
        &[]
    }
}
