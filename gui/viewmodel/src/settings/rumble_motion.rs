use nerust_settings_core::{i18n::UiText, model::local::RumbleTarget};

use super::{
    dto::{ChoiceView, RumbleMotionView},
    editor::SettingsEditor,
    property::ReadOnlyObservableProperty,
};

/// Sub-view model for the Rumble & Motion tab: motion sensor enable,
/// rumble switch, strength, and output target. All four rows persist
/// ahead of the gamepad backend and apply once it connects.
#[derive(Clone)]
pub struct RumbleMotionViewModel {
    editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<RumbleMotionView>,
}

impl RumbleMotionViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("rumble_motion", initial, project_view);
        Self {
            editor: editor.clone(),
            view,
        }
    }

    pub fn set_motion_enabled(&self, enabled: bool) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            state.draft_mut().local.rumble_motion.motion_enabled = enabled;
            Ok(())
        })
    }

    pub fn set_rumble_enabled(&self, enabled: bool) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            state.draft_mut().local.rumble_motion.rumble_enabled = enabled;
            Ok(())
        })
    }

    pub fn set_rumble_strength(&self, percent: u8) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            // Strength is a plain 0-100 percent (unlike the stepped
            // stick deadzone); the slider bounds match validation.
            state
                .draft_mut()
                .local
                .rumble_motion
                .rumble_strength_percent = percent.min(100);
            Ok(())
        })
    }

    pub fn set_rumble_target(
        &self,
        target: RumbleTarget,
    ) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            state.draft_mut().local.rumble_motion.rumble_target = target;
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> RumbleMotionView {
    let i18n = super::state::display_i18n(state);
    let settings = &state.draft.local.rumble_motion;
    RumbleMotionView {
        motion_enabled: settings.motion_enabled,
        rumble_enabled: settings.rumble_enabled,
        rumble_strength_percent: settings.rumble_strength_percent,
        rumble_target: settings.rumble_target,
        rumble_target_choices: [
            (RumbleTarget::Auto, UiText::Auto),
            (RumbleTarget::Handset, UiText::Handset),
            (RumbleTarget::Controller, UiText::Controller),
        ]
        .into_iter()
        .map(|(value, label)| ChoiceView {
            value,
            label: i18n.text(label),
        })
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_core::model::local::RumbleTarget;

    use crate::settings::test_support::test_vm;

    #[test]
    fn rumble_motion_projects_defaults_and_accepts_edits() {
        let vm = test_vm();
        let view = vm.rumble_motion.view.get();
        assert!(view.motion_enabled);
        assert!(view.rumble_enabled);
        assert_eq!(view.rumble_strength_percent, 100);
        assert_eq!(view.rumble_target, RumbleTarget::Auto);
        assert_eq!(view.rumble_target_choices.len(), 3);
        vm.rumble_motion.set_motion_enabled(false).unwrap();
        assert!(!vm.rumble_motion.view.get().motion_enabled);
        vm.rumble_motion.set_rumble_enabled(false).unwrap();
        assert!(!vm.rumble_motion.view.get().rumble_enabled);
        vm.rumble_motion.set_rumble_strength(25).unwrap();
        assert_eq!(vm.rumble_motion.view.get().rumble_strength_percent, 25);
        vm.rumble_motion
            .set_rumble_target(RumbleTarget::Controller)
            .unwrap();
        assert_eq!(
            vm.rumble_motion.view.get().rumble_target,
            RumbleTarget::Controller
        );
    }
}
