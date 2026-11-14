use nerust_input_traits::Key;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{I18n, UiText},
};

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
            // Row-level reset: both columns clear together, so a pad-only
            // row cannot strand an invisible keyboard twin. Game binding
            // rows never carry pads, so only shortcuts take the pad clear.
            // Per-pad mapping rows clear their own override the same way.
            nerust_settings_core::editor::apply_capture_target(state.draft_mut(), target, None);
            if matches!(
                target,
                CaptureTarget::Shortcut(_) | CaptureTarget::PadBinding { .. }
            ) {
                nerust_settings_core::editor::apply_capture_pad_target(
                    state.draft_mut(),
                    target,
                    None,
                );
            }
            state.capture_target = None;
            Ok(())
        })
    }

    /// Clear one shortcut column, keeping its twin: per-device settings
    /// rows address the keyboard and the pad independently.
    pub fn clear_key(&self, target: &CaptureTarget) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            nerust_settings_core::editor::apply_capture_target(state.draft_mut(), target, None);
            state.capture_target = None;
            Ok(())
        })
    }

    /// Clear one shortcut pad column, keeping its keyboard twin.
    pub fn clear_pad(&self, target: &CaptureTarget) -> Result<(), ViewModelError> {
        self.editor.transact(|state| {
            nerust_settings_core::editor::apply_capture_pad_target(state.draft_mut(), target, None);
            state.capture_target = None;
            Ok(())
        })
    }

    pub fn apply_captured_key(&self, key: Key) {
        let target = self.editor.current().capture_target.clone();
        let Some(target) = target else { return };
        let _ = self.editor.transact(|state| {
            nerust_settings_core::editor::apply_capture_target(
                state.draft_mut(),
                &target,
                Some(key),
            );
            state.capture_target = None;
            Ok(())
        });
    }

    pub fn apply_captured_pad(&self, button: nerust_input_traits::StandardButton) {
        let target = self.editor.current().capture_target.clone();
        let Some(target) = target else { return };
        let _ = self.editor.transact(|state| {
            nerust_settings_core::editor::apply_capture_pad_target(
                state.draft_mut(),
                &target,
                Some(button),
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
    let i18n = I18n::resolve_os(state.draft.shared.general.language);
    CaptureStateView {
        target: state.capture_target.clone(),
        prompt: state
            .capture_target
            .as_ref()
            .map(|_| i18n.text(UiText::CapturePrompt))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use nerust_input_traits::Key;
    use nerust_settings_core::model::input::ShortcutAction;

    use super::CaptureTarget;
    use crate::settings::test_support::test_vm;

    #[test]
    fn start_capture_sets_target() {
        let vm = test_vm();
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        vm.capture.start_capture(target.clone()).unwrap();
        let view = vm.capture.view.get();
        assert_eq!(view.target, Some(target));
    }

    #[test]
    fn apply_captured_key_clears_target() {
        let vm = test_vm();
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        vm.capture.start_capture(target).unwrap();
        vm.capture.apply_captured_key(Key::Space);
        let view = vm.capture.view.get();
        assert!(view.target.is_none());
    }

    #[test]
    fn cancel_capture_clears_target() {
        let vm = test_vm();
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        vm.capture.start_capture(target).unwrap();
        vm.capture.cancel_capture();
        let view = vm.capture.view.get();
        assert!(view.target.is_none());
    }

    #[test]
    fn clear_binding_clears_target() {
        let vm = test_vm();
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        vm.capture.start_capture(target.clone()).unwrap();
        vm.capture.clear_binding(&target).unwrap();
        let view = vm.capture.view.get();
        assert!(view.target.is_none());
    }
}
