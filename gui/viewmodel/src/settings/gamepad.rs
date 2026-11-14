use nerust_input_traits::MAX_PADS;
use nerust_settings_core::i18n::{I18n, UiText, pad_label, pad_triggers_label};
use nerust_settings_core::model::input::{DEADZONE_MAX_PERCENT, DEADZONE_STEPS};

use super::{
    dto::{ChoiceView, GamepadTabView, PadPortView},
    editor::SettingsEditor,
    property::ReadOnlyObservableProperty,
};

/// Sub-view model for the per-pad gamepads screen: each
/// connection-order pad carries its own deadzone plus its player port.
/// Projected once per editor state, global like shortcuts (pads are not
/// per-system, though their button overrides are).
#[derive(Clone)]
pub struct GamepadViewModel {
    editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<GamepadTabView>,
}

impl GamepadViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("gamepads", initial, project_view);
        Self {
            editor: editor.clone(),
            view,
        }
    }

    /// Stick deadzone for one connection-order pad in percent, clamped
    /// to the schema range. Sticks vary per unit, so the deadzone rides
    /// the pad, never a global.
    pub fn set_deadzone(
        &self,
        pad: usize,
        percent: u8,
    ) -> Result<(), super::editor::ViewModelError> {
        self.update_pad(pad, |port_state| {
            port_state.deadzone_percent = percent.min(DEADZONE_MAX_PERCENT);
        })
    }

    /// Port for one connection-order pad (`None` parks it). Pads past the
    /// table never reach here (their rows do not exist), so the guard is
    /// defense against a tampered caller, not a normal path.
    pub fn set_pad_port(
        &self,
        pad: usize,
        port: Option<usize>,
    ) -> Result<(), super::editor::ViewModelError> {
        self.update_pad(pad, |port_state| port_state.slot = port)
    }

    /// Trigger doubling for one pad: shoulders when true, analog-only
    /// when false. Same guard as the port setter.
    pub fn set_trigger_alias(
        &self,
        pad: usize,
        as_shoulders: bool,
    ) -> Result<(), super::editor::ViewModelError> {
        self.update_pad(pad, |port_state| {
            port_state.triggers_as_shoulders = as_shoulders
        })
    }

    /// One guarded write to a pad's port state; both setters share the
    /// range check so neither duplicates it.
    fn update_pad(
        &self,
        pad: usize,
        update: impl FnOnce(&mut nerust_settings_core::model::input::PadPort),
    ) -> Result<(), super::editor::ViewModelError> {
        if pad >= MAX_PADS {
            return Err(super::editor::ViewModelError::UnknownPad(pad));
        }
        self.editor.transact(|state| {
            update(&mut state.draft_mut().shared.input.gamepad.pad_ports[pad]);
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> GamepadTabView {
    let i18n = I18n::resolve_os(state.draft.shared.general.language);
    let gamepad = &state.draft.shared.input.gamepad;
    // Port and trigger options are identical for every pad: build once,
    // clone per row.
    let port_choices: Vec<ChoiceView<Option<usize>>> = [
        (None, i18n.text(UiText::NotAssigned)),
        (Some(0), i18n.text(UiText::PlayerOne)),
        (Some(1), i18n.text(UiText::PlayerTwo)),
    ]
    .into_iter()
    .map(|(port, label)| ChoiceView { value: port, label })
    .collect();
    let trigger_choices: Vec<ChoiceView<bool>> = [
        (false, i18n.text(UiText::Off)),
        (true, i18n.text(UiText::On)),
    ]
    .into_iter()
    .map(|(alias, label)| ChoiceView {
        value: alias,
        label,
    })
    .collect();
    GamepadTabView {
        label: i18n.text(UiText::Gamepads),
        deadzone_choices: DEADZONE_STEPS
            .iter()
            .map(|step| ChoiceView {
                value: *step,
                label: format!("{step}%"),
            })
            .collect(),
        pads: (0..MAX_PADS)
            .map(|index| {
                let port = &gamepad.pad_ports[index];
                PadPortView {
                    index,
                    label: pad_label(i18n.language(), index),
                    port: port.slot,
                    choices: port_choices.clone(),
                    trigger_label: pad_triggers_label(i18n.language(), index),
                    trigger_alias: port.triggers_as_shoulders,
                    trigger_choices: trigger_choices.clone(),
                    deadzone: port.deadzone_percent,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::test_support::test_vm;

    #[test]
    fn gamepads_project_defaults_and_accept_edits() {
        use nerust_settings_core::model::input::DEFAULT_DEADZONE_PERCENT;
        let vm = test_vm();
        let view = vm.gamepads.view.get();
        assert_eq!(view.pads.len(), nerust_input_traits::MAX_PADS);
        assert_eq!(view.pads[0].deadzone, DEFAULT_DEADZONE_PERCENT);
        assert_eq!(view.pads[0].port, Some(0));
        assert_eq!(view.pads[2].port, None);
        vm.gamepads.set_deadzone(0, 25).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[0].deadzone, 25);
        vm.gamepads.set_deadzone(0, 200).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[0].deadzone, 90);
        // Pads tune independently.
        vm.gamepads.set_deadzone(1, 10).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[1].deadzone, 10);
        assert_eq!(vm.gamepads.view.get().pads[0].deadzone, 90);
        vm.gamepads.set_pad_port(2, Some(1)).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[2].port, Some(1));
        assert!(vm.gamepads.set_pad_port(9, Some(0)).is_err());
        assert!(vm.gamepads.set_deadzone(9, 10).is_err());
    }
}
