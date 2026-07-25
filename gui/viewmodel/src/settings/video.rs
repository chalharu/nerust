use nerust_gui_settings::local::ScalingMode;
use nerust_settings_core::i18n::{UiText, text as ui_text};

use super::{
    dto::{ChoiceView, VideoView},
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

#[derive(Clone)]
pub struct VideoSettingsViewModel {
    _editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<VideoView>,
}

impl VideoSettingsViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("video", initial, project_view);
        Self {
            _editor: editor.clone(),
            view,
        }
    }

    pub fn set_fullscreen_default(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft.local.video.window.fullscreen_default = value;
            Ok(())
        })
    }

    pub fn set_scaling(&self, value: ScalingMode) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft.local.video.window.scaling = value;
            Ok(())
        })
    }

    pub fn set_vsync(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft.local.video.presentation.vsync = value;
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> VideoView {
    let lang = state.draft.shared.general.language;
    VideoView {
        fullscreen_default: state.draft.local.video.window.fullscreen_default,
        scaling: state.draft.local.video.window.scaling,
        scaling_choices: vec![
            ChoiceView {
                value: ScalingMode::FitToWindow,
                label: ui_text(lang, UiText::FitToWindow).to_string(),
            },
            ChoiceView {
                value: ScalingMode::X1,
                label: "1x".into(),
            },
            ChoiceView {
                value: ScalingMode::X2,
                label: "2x".into(),
            },
            ChoiceView {
                value: ScalingMode::X3,
                label: "3x".into(),
            },
            ChoiceView {
                value: ScalingMode::X4,
                label: "4x".into(),
            },
            ChoiceView {
                value: ScalingMode::X5,
                label: "5x".into(),
            },
        ],
        vsync: state.draft.local.video.presentation.vsync,
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::test_support::test_vm;
    use nerust_gui_settings::local::ScalingMode;

    #[test]
    fn set_fullscreen_default_updates_projection() {
        let vm = test_vm();
        vm.video.set_fullscreen_default(true).unwrap();
        let view = vm.video.view.get();
        assert!(view.fullscreen_default);
    }

    #[test]
    fn set_scaling_updates_projection() {
        let vm = test_vm();
        vm.video.set_scaling(ScalingMode::X3).unwrap();
        let view = vm.video.view.get();
        assert_eq!(view.scaling, ScalingMode::X3);
    }

    #[test]
    fn set_vsync_updates_projection() {
        let vm = test_vm();
        vm.video.set_vsync(true).unwrap();
        let view = vm.video.view.get();
        assert!(view.vsync);
    }

    #[test]
    fn scaling_choices_include_all_modes() {
        let vm = test_vm();
        let view = vm.video.view.get();
        assert_eq!(view.scaling_choices.len(), 6);
    }

    #[test]
    fn set_same_scaling_is_noop() {
        let vm = test_vm();
        let rev_before = vm.revision.get();
        vm.video.set_scaling(ScalingMode::FitToWindow).unwrap();
        assert_eq!(vm.revision.get(), rev_before, "revision should not advance");
    }
}
