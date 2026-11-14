//! Per-pad settings screens: one `pad.<n>` screen per connected pad.
//!
//! Pure projection like the rest of `settings_views`: reads the view model,
//! the draft snapshot, and the vault device truth, and returns wire rows.
//! No mutation.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::UiText,
    input::slot_port_index,
    model::input::{DEADZONE_STEPS, pad_game_binding_conflicts},
};

use super::devices::player_label;
use super::projection::PLAYER_PORTS;
use super::wire::{
    GamepadInfo, I18n, RowWire, SectionWire, button_row, capture_row, encode_target, group_row,
    header_row, info_row, number_row, toggle_row,
};

/// Per-pad settings screens: one `pad.<n>` screen per connected pad
/// with its own deadzone, trigger doubling, and per-system button
/// mapping. The list is variable: each connected pad names its own
/// screen, so Pad 1 is always the first-seen pad while it stays
/// connected. Ports are player slots, not devices: the connection-order
/// table (not device ids) is what persists. Unconnected pads hide; their
/// assignment and overrides stay in the tables and reappear on reconnect.
pub(super) fn build_pad_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    let reason = i18n.text(UiText::InputDeviceRequired);
    let gamepad = &snapshot.shared.input.gamepad;
    gamepads
        .iter()
        .take(nerust_input_traits::MAX_PADS)
        .enumerate()
        .map(|(index, info)| {
            let pad_id = format!("pad.{}", index + 1);
            let port = gamepad.pad_ports.get(index);
            // Assigned player slot, if any. Mapping rows below scope to
            // this pad's own player: other players' rows carry defaults
            // this pad can never drive, and edits there land overrides
            // that gameplay parks, so they only mislead.
            let slot = port.and_then(|pad| pad.slot);
            // Tapping the slot jumps to the Devices screen that owns the
            // assignment radio: the pad screen shows mapping, Devices
            // shows belonging. Unassigned pads keep the guidance row below
            // instead: no single Devices screen owns them.
            let slot_row = match slot.filter(|slot| PLAYER_PORTS.get(*slot).is_some()) {
                // Title names the player (the assignment this screen
                // drives); the text confirms which pad this screen is.
                // The old title said "Gamepad" while the value said the
                // player, answering neither "which" nor "whose".
                Some(slot) => {
                    let mut row = group_row(
                        &format!("{pad_id}.slot"),
                        player_label(slot, i18n),
                        String::new(),
                        &format!("devices.{}", PLAYER_PORTS[slot]),
                    );
                    row.text = info.name.clone();
                    row
                }
                None => info_row(
                    &format!("{pad_id}.slot"),
                    i18n.text(UiText::GamepadDevice),
                    String::new(),
                    i18n.text(UiText::NotAssigned),
                ),
            };
            let mut rows = vec![
                slot_row,
                number_row(
                    &format!("{pad_id}.deadzone"),
                    i18n.text(UiText::Deadzone),
                    i18n.text(UiText::DescDeadzone),
                    i64::from(port.map_or(
                        nerust_settings_core::model::input::DEFAULT_DEADZONE_PERCENT,
                        |pad| pad.deadzone_percent,
                    )),
                    i64::from(DEADZONE_STEPS[1] - DEADZONE_STEPS[0]),
                    i64::from(DEADZONE_STEPS[0]),
                    i64::from(DEADZONE_STEPS[DEADZONE_STEPS.len() - 1]),
                    "%",
                ),
                toggle_row(
                    &format!("{pad_id}.triggers"),
                    nerust_settings_core::i18n::pad_triggers_label(i18n.language(), index),
                    String::new(),
                    i18n,
                    port.is_some_and(|pad| pad.triggers_as_shoulders),
                ),
                toggle_row(
                    &format!("{pad_id}.stick_left"),
                    i18n.text(UiText::LeftStickAsDpad),
                    String::new(),
                    i18n,
                    port.is_none_or(|pad| pad.left_stick_as_dpad),
                ),
                toggle_row(
                    &format!("{pad_id}.stick_right"),
                    i18n.text(UiText::RightStickAsDpad),
                    String::new(),
                    i18n,
                    port.is_none_or(|pad| pad.right_stick_as_dpad),
                ),
            ];
            // Unassigned pads park every press, so mapping rows would
            // only collect latent overrides: point at Devices instead.
            // Assigned pads list only their own player's controls (all
            // systems); other players' attachments never fire from here.
            let Some(slot) = slot else {
                rows.push(info_row(
                    &format!("{pad_id}.unassigned"),
                    i18n.text(UiText::KeyMapping),
                    i18n.text(UiText::DescPadUnassigned),
                    String::new(),
                ));
                rows.push(button_row(
                    &format!("reset.{pad_id}"),
                    i18n.text(UiText::ResetScreen),
                    i18n.text(UiText::DescPadReset),
                ));
                return SectionWire {
                    id: pad_id.clone(),
                    label: info.name.clone(),
                    sub: i18n.text(UiText::Gamepads),
                    rows,
                    visible: false,
                };
            };
            for input in vm.inputs() {
                let view = input.view.get();
                let system = view.system_id.to_string();
                let mut system_rows = Vec::new();
                for section in &view.sections {
                    let mut section_rows = Vec::new();
                    for binding in &section.rows {
                        // Per-pad mapping mirrors the game rows: same
                        // attachment/control, routed to this pad.
                        let (Some(attachment), Some(control)) =
                            (binding.target.attachment(), binding.target.control())
                        else {
                            continue;
                        };
                        // Attachments name their player port
                        // (`<sys>.attachment.playerN`); unclassifiable
                        // attachments fail open so a factory addition
                        // never silently drops rows here.
                        if slot_port_index(attachment).is_some_and(|port| port != slot) {
                            continue;
                        }
                        let target = CaptureTarget::PadBinding {
                            system: view.system_id.clone_box(),
                            pad: index,
                            attachment: attachment.to_string(),
                            control: control.to_string(),
                        };
                        let value = input.pad_row_value(&target, binding.pad_default);
                        let text = match &value {
                            BindingValueView::Unbound(label)
                            | BindingValueView::Bound(label)
                            | BindingValueView::Capturing(label) => label.clone(),
                        };
                        section_rows.push(capture_row(
                            &format!("{pad_id}.{system}.{attachment}.{control}",),
                            binding.label.clone(),
                            String::new(),
                            text,
                            &encode_target(&target, &system),
                            true,
                            &reason,
                        ));
                    }
                    if section_rows.is_empty() {
                        continue;
                    }
                    system_rows.push(header_row(section.label.clone()));
                    system_rows.append(&mut section_rows);
                }
                if system_rows.is_empty() {
                    continue;
                }
                rows.push(header_row(view.label.clone()));
                rows.append(&mut system_rows);
                // One button driving two controls never fires the second:
                // surface it on the pad's own screen, next to the rows.
                // Scoped like the rows above: hidden attachments have no
                // rows to disambiguate against.
                if let Some(settings) = snapshot.shared.input.systems.get(&view.system_id)
                    && let Some(entries) = settings.pad_bindings.get(index)
                {
                    rows.extend(conflict_rows(&pad_id, &system, slot, entries, i18n));
                }
            }
            rows.push(button_row(
                &format!("reset.{pad_id}"),
                i18n.text(UiText::ResetScreen),
                i18n.text(UiText::DescPadReset),
            ));
            SectionWire {
                id: pad_id.clone(),
                label: info.name.clone(),
                sub: i18n.text(UiText::Gamepads),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// One button driving two in-scope controls: a warning row per button.
/// Out-of-scope controls never render, and a lone survivor is no conflict
/// on this screen.
fn conflict_rows(
    pad_id: &str,
    system: &str,
    slot: usize,
    entries: &[nerust_settings_core::model::input::PadBinding],
    i18n: I18n,
) -> Vec<RowWire> {
    let mut rows = Vec::new();
    for (button, controls) in pad_game_binding_conflicts(entries) {
        let shown: Vec<_> = controls
            .iter()
            .filter(|(attachment, _)| {
                !slot_port_index(attachment.as_str()).is_some_and(|port| port != slot)
            })
            .collect();
        if shown.len() < 2 {
            continue;
        }
        let name = i18n.pad_button_label(button);
        rows.push(info_row(
            &format!("{pad_id}.{system}.conflict.{name}"),
            name.clone(),
            shown
                .iter()
                .map(|(attachment, control)| format!("{attachment}.{control}"))
                .collect::<Vec<_>>()
                .join(", "),
            String::new(),
        ));
    }
    rows
}
