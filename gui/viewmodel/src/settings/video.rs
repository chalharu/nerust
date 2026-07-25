use nerust_gui_settings::local::ScalingMode;

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
    VideoView {
        fullscreen_default: state.draft.local.video.window.fullscreen_default,
        scaling: state.draft.local.video.window.scaling,
        scaling_choices: vec![
            ChoiceView {
                value: ScalingMode::FitToWindow,
                label: "Fit to Window".into(),
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
