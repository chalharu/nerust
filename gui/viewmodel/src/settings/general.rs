use nerust_gui_settings::{language::AppLanguage, shared::StoragePolicy};
use std::path::PathBuf;

use super::{
    dto::{ChoiceView, GeneralView},
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

/// Sub-view model for the General settings page.
#[derive(Clone)]
pub struct GeneralSettingsViewModel {
    editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<GeneralView>,
}

impl GeneralSettingsViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("general", initial, project_view);
        Self {
            editor: editor.clone(),
            view,
        }
    }

    pub fn set_language(&self, value: AppLanguage) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            state.draft.shared.general.language = value;
            Ok(())
        })
    }

    pub fn set_storage_policy(&self, value: StoragePolicy) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            state.draft.shared.persistence.storage_policy = value;
            Ok(())
        })
    }

    pub fn set_storage_directory(&self, value: Option<PathBuf>) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            state.draft.shared.persistence.storage_directory = value;
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> GeneralView {
    let lang = state.draft.shared.general.language;
    GeneralView {
        language: lang,
        language_choices: vec![
            ChoiceView {
                value: AppLanguage::SystemDefault,
                label: "System Default".into(),
            },
            ChoiceView {
                value: AppLanguage::Japanese,
                label: "Japanese".into(),
            },
            ChoiceView {
                value: AppLanguage::English,
                label: "English".into(),
            },
        ],
        storage_policy: state.draft.shared.persistence.storage_policy,
        storage_policy_choices: vec![
            ChoiceView {
                value: StoragePolicy::Sidecar,
                label: "Sidecar".into(),
            },
            ChoiceView {
                value: StoragePolicy::AppSharedData,
                label: "App Shared Data".into(),
            },
            ChoiceView {
                value: StoragePolicy::CustomDirectory,
                label: "Custom Directory".into(),
            },
        ],
        storage_directory: state
            .draft
            .shared
            .persistence
            .storage_directory
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        show_storage_directory: matches!(
            state.draft.shared.persistence.storage_policy,
            StoragePolicy::CustomDirectory
        ),
    }
}
