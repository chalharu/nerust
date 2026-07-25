use nerust_gui_settings::{language::AppLanguage, shared::StoragePolicy};
use nerust_settings_core::i18n::{UiText, text as ui_text};
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

#[cfg(test)]
mod tests {
    use crate::settings::test_support::test_vm;
    use nerust_gui_settings::{language::AppLanguage, shared::StoragePolicy};

    #[test]
    fn set_language_updates_projection() {
        let vm = test_vm();
        vm.general.set_language(AppLanguage::Japanese).unwrap();
        let view = vm.general.view.get();
        assert_eq!(view.language, AppLanguage::Japanese);
    }

    #[test]
    fn set_storage_policy_updates_projection() {
        let vm = test_vm();
        vm.general
            .set_storage_policy(StoragePolicy::CustomDirectory)
            .unwrap();
        let view = vm.general.view.get();
        assert_eq!(view.storage_policy, StoragePolicy::CustomDirectory);
        assert!(view.show_storage_directory);
    }

    #[test]
    fn set_storage_directory_updates_projection() {
        let vm = test_vm();
        let p = std::path::PathBuf::from("/tmp/test");
        vm.general.set_storage_directory(Some(p.clone())).unwrap();
        let view = vm.general.view.get();
        assert_eq!(view.storage_directory, "/tmp/test");
    }

    #[test]
    fn language_choices_are_localized() {
        let vm = test_vm();
        let view = vm.general.view.get();
        assert_eq!(view.language_choices.len(), 3);
    }

    #[test]
    fn set_same_language_is_noop() {
        let vm = test_vm();
        let rev_before = vm.revision.get();
        vm.general.set_language(AppLanguage::SystemDefault).unwrap();
        assert_eq!(vm.revision.get(), rev_before, "revision should not advance");
    }
}

#[allow(clippy::items_after_test_module)]
fn project_view(state: &super::EditorState) -> GeneralView {
    let lang = state.draft.shared.general.language;
    GeneralView {
        language: lang,
        language_choices: vec![
            ChoiceView {
                value: AppLanguage::SystemDefault,
                label: ui_text(lang, UiText::SystemDefault).to_string(),
            },
            ChoiceView {
                value: AppLanguage::Japanese,
                label: ui_text(lang, UiText::Japanese).to_string(),
            },
            ChoiceView {
                value: AppLanguage::English,
                label: ui_text(lang, UiText::English).to_string(),
            },
        ],
        storage_policy: state.draft.shared.persistence.storage_policy,
        storage_policy_choices: vec![
            ChoiceView {
                value: StoragePolicy::Sidecar,
                label: ui_text(lang, UiText::Sidecar).to_string(),
            },
            ChoiceView {
                value: StoragePolicy::AppSharedData,
                label: ui_text(lang, UiText::AppSharedData).to_string(),
            },
            ChoiceView {
                value: StoragePolicy::CustomDirectory,
                label: ui_text(lang, UiText::CustomDirectory).to_string(),
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
