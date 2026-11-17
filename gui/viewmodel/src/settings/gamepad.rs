use nerust_settings_core::i18n::UiText;
use nerust_settings_core::labels::pad_label;
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
        self.update_pad(pad, |ports, pad| {
            ports[pad].deadzone_percent = percent.min(DEADZONE_MAX_PERCENT);
        })
    }

    /// Port for one connection-order pad (`None` parks it). Assigning a
    /// port clears the same port on every other pad (exclusive assign:
    /// one player, one pad), mirroring the Android radio semantics.
    /// Pads past the table never reach here (their rows do not exist),
    /// so the guard is defense against a tampered caller, not a normal path.
    pub fn set_pad_port(
        &self,
        pad: usize,
        port: Option<usize>,
    ) -> Result<(), super::editor::ViewModelError> {
        self.update_pad(pad, |ports, pad| {
            if let Some(port) = port {
                for (index, other) in ports.iter_mut().enumerate() {
                    if index != pad && other.slot == Some(port) {
                        other.slot = None;
                    }
                }
            }
            ports[pad].slot = port;
        })
    }

    /// Physical dpad enable for one pad: `false` parks the hat (sticks
    /// still drive through their substitutes), `true` restores it.
    /// Same guard as the port setter.
    pub fn set_dpad_enabled(
        &self,
        pad: usize,
        enabled: bool,
    ) -> Result<(), super::editor::ViewModelError> {
        self.update_pad(pad, |ports, pad| {
            ports[pad].dpad_enabled = enabled;
        })
    }

    /// One write to a pad's port state; open tables grow on demand so
    /// a newly connected pad is configurable the moment its screen
    /// exists. Both setters share the growth so neither duplicates it.
    /// The closure sees the whole table plus its own index, so port
    /// assignment can clear same-port rivals.
    fn update_pad(
        &self,
        pad: usize,
        update: impl FnOnce(&mut Vec<nerust_settings_core::model::input::PadPort>, usize),
    ) -> Result<(), super::editor::ViewModelError> {
        self.editor.transact(|state| {
            let ports = &mut state.draft_mut().shared.input.gamepad.pad_ports;
            if ports.len() <= pad {
                ports.resize_with(
                    pad + 1,
                    nerust_settings_core::model::input::PadPort::default,
                );
            }
            update(ports, pad);
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> GamepadTabView {
    let i18n = super::state::display_i18n(state);
    let gamepad = &state.draft.shared.input.gamepad;
    // Port options are identical for every pad: build once, clone per row.
    let port_choices: Vec<ChoiceView<Option<usize>>> = [
        (None, i18n.text(UiText::NotAssigned)),
        (Some(0), i18n.text(UiText::PlayerOne)),
        (Some(1), i18n.text(UiText::PlayerTwo)),
    ]
    .into_iter()
    .map(|(port, label)| ChoiceView { value: port, label })
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
        pads: (0..gamepad.pad_ports.len())
            .map(|index| {
                let port = &gamepad.pad_ports[index];
                PadPortView {
                    index,
                    label: pad_label(i18n, index),
                    port: port.slot,
                    choices: port_choices.clone(),
                    deadzone: port.deadzone_percent,
                    dpad_enabled: port.dpad_enabled,
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
        assert_eq!(
            view.pads.len(),
            nerust_settings_core::model::input::GamepadSettings::default()
                .pad_ports
                .len()
        );
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
        // Open tables: addressing a new pad grows the settings instead of
        // erroring, so a fifth pad is configurable the moment it appears.
        vm.gamepads.set_pad_port(9, Some(0)).unwrap();
        assert_eq!(vm.gamepads.view.get().pads.len(), 10);
        assert_eq!(vm.gamepads.view.get().pads[9].port, Some(0));
        vm.gamepads.set_deadzone(9, 10).unwrap();
        assert_eq!(vm.gamepads.view.get().pads[9].deadzone, 10);
    }

    #[test]
    fn set_pad_port_clears_same_port_rivals() {
        let vm = test_vm();
        // Default fixture parks pads 0 and 1 on players 1 and 2.
        assert_eq!(vm.gamepads.view.get().pads[0].port, Some(0));
        assert_eq!(vm.gamepads.view.get().pads[1].port, Some(1));
        // Taking player 1 with pad 1 parks pad 0: one player, one pad.
        vm.gamepads.set_pad_port(1, Some(0)).unwrap();
        let view = vm.gamepads.view.get();
        assert_eq!(view.pads[1].port, Some(0));
        assert_eq!(view.pads[0].port, None);
        // Parking never clears rivals.
        vm.gamepads.set_pad_port(1, None).unwrap();
        let view = vm.gamepads.view.get();
        assert_eq!(view.pads[1].port, None);
        assert_eq!(view.pads[0].port, None);
    }
}
