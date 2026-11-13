use nerust_input_traits::MAX_PADS;
use nerust_settings_core::i18n::{I18n, UiText};

use super::{
    dto::{ChoiceView, GamepadTabView, PadPortView},
    editor::SettingsEditor,
    property::ReadOnlyObservableProperty,
};

/// Sub-view model for the global gamepads screen: stick deadzone plus the
/// connection-order port table. Projected once per editor state, global
/// like shortcuts (pads are not per-system).
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

    /// Stick deadzone in percent, clamped to the schema range.
    pub fn set_deadzone(&self, percent: u8) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            state.draft_mut().shared.input.gamepad.deadzone_percent = percent.min(90);
            Ok(())
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
        if pad >= MAX_PADS {
            return Err(super::editor::ViewModelError::UnknownPad(pad));
        }
        self.editor.transact(|state| {
            state.draft_mut().shared.input.gamepad.pad_ports[pad].slot = port;
            Ok(())
        })
    }

    /// Trigger doubling for one pad: shoulders when true, analog-only
    /// when false. Same guard as the port setter.
    pub fn set_trigger_alias(
        &self,
        pad: usize,
        as_shoulders: bool,
    ) -> Result<(), super::editor::ViewModelError> {
        if pad >= MAX_PADS {
            return Err(super::editor::ViewModelError::UnknownPad(pad));
        }
        self.editor.transact(|state| {
            state.draft_mut().shared.input.gamepad.pad_ports[pad].triggers_as_shoulders =
                as_shoulders;
            Ok(())
        })
    }
}

/// Deadzone steps offered on both frontends. Sliders would fake precision
/// the integer-percent schema cannot keep; 5-point steps stay honest.
const DEADZONE_STEPS: [u8; 7] = [0, 5, 10, 15, 20, 25, 30];

fn project_view(state: &super::EditorState) -> GamepadTabView {
    let i18n = I18n::resolve_os(state.draft.shared.general.language);
    let gamepad = &state.draft.shared.input.gamepad;
    GamepadTabView {
        label: i18n.text(UiText::Gamepads),
        deadzone_label: i18n.text(UiText::Deadzone),
        deadzone_desc: i18n.text(UiText::DescDeadzone),
        deadzone: gamepad.deadzone_percent,
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
                    label: format!("Pad {}", index + 1),
                    port: port.slot,
                    choices: [
                        (None, i18n.text(UiText::Off)),
                        (Some(0), i18n.text(UiText::PlayerOne)),
                        (Some(1), i18n.text(UiText::PlayerTwo)),
                    ]
                    .into_iter()
                    .map(|(port, label)| ChoiceView { value: port, label })
                    .collect(),
                    trigger_label: format!("Pad {} triggers", index + 1),
                    trigger_alias: port.triggers_as_shoulders,
                    trigger_choices: [
                        (false, i18n.text(UiText::Off)),
                        (true, i18n.text(UiText::On)),
                    ]
                    .into_iter()
                    .map(|(alias, label)| ChoiceView {
                        value: alias,
                        label,
                    })
                    .collect(),
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
        let vm = test_vm();
        let view = vm.gamepads.view.get();
        assert_eq!(view.pads.len(), nerust_input_traits::MAX_PADS);
        assert_eq!(view.deadzone, 15);
        assert_eq!(view.pads[0].port, Some(0));
        assert_eq!(view.pads[2].port, None);
        vm.gamepads.set_deadzone(25).unwrap();
        assert_eq!(vm.gamepads.view.get().deadzone, 25);
        vm.gamepads.set_deadzone(200).unwrap();
        assert_eq!(vm.gamepads.view.get().deadzone, 90);
        vm.gamepads.set_pad_port(2, Some(1)).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[2].port, Some(1));
        assert!(vm.gamepads.set_pad_port(9, Some(0)).is_err());
    }
}
