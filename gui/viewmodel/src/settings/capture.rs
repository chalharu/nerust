use nerust_gui_shell::settings::{
    editor::CaptureTarget,
    i18n::{UiText, text as ui_text},
};
use nerust_keyboard::Key;

use super::{
    dto::CaptureStateView,
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

#[derive(Clone)]
pub struct CaptureViewModel {
    editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<CaptureStateView>,
}

impl CaptureViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("capture", initial, project_view);
        Self {
            editor: editor.clone(),
            view,
        }
    }

    pub fn start_capture(&self, target: CaptureTarget) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            state.capture_target = Some(target);
            Ok(())
        })
    }

    pub fn clear_binding(&self, target: &CaptureTarget) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            nerust_gui_shell::settings::editor::apply_capture_target(
                &mut state.draft,
                target,
                None,
            );
            state.capture_target = None;
            Ok(())
        })
    }

    pub fn apply_captured_key(&self, key: Key) {
        let target = self.editor.current().capture_target.clone();
        let Some(target) = target else { return };
        let _ = self.editor.transact(|state| {
            nerust_gui_shell::settings::editor::apply_capture_target(
                &mut state.draft,
                &target,
                Some(key),
            );
            state.capture_target = None;
            Ok(())
        });
    }

    pub fn cancel_capture(&self) {
        let _ = self.editor.transact(|state| {
            state.capture_target = None;
            Ok(())
        });
    }
}

fn project_view(state: &super::EditorState) -> CaptureStateView {
    CaptureStateView {
        target: state.capture_target.clone(),
        prompt: state
            .capture_target
            .as_ref()
            .map(|_| {
                ui_text(state.draft.shared.general.language, UiText::CapturePrompt).to_string()
            })
            .unwrap_or_default(),
    }
}
