//! Per-pad settings screens: one `pad.<n>` screen per connected pad.
//!
//! Pure projection like the rest of `settings_views`: reads the view model,
//! the draft snapshot, and the vault device truth, and returns wire rows.
//! No mutation.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_input_traits::port_index_for_slot;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::UiText,
    model::input::{DEADZONE_STEPS, pad_game_binding_conflicts},
};

use super::wire::{
    GamepadInfo, I18n, PLAYER_PORTS, RowWire, SectionWire, button_row, capture_row, encode_target,
    header_row, info_row, number_row, player_label, toggle_row,
};

/// Per-pad settings screens: one `pad.<n>` screen per connected pad
/// with its own deadzone, hat switch, and per-system button mapping.
/// Stick/trigger routing is automatic (the live console declares analog
/// controls or not), so no toggles: only the hat stays manual. The list
/// is variable: each connected pad names its own screen, so Pad 1 is
/// always the first-seen pad while it stays connected. Ports are player
/// slots, not devices: the connection-order table (not device ids) is
/// what persists. Unconnected pads hide; their assignment and overrides
/// stay in the tables and reappear on reconnect.
pub(super) fn build_pad_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    let reason = i18n.text(UiText::InputDeviceRequired);
    let gamepad = &snapshot.shared.input.gamepad;
    let mut out = Vec::new();
    for (index, info) in gamepads
        .iter()
        .take(nerust_input_traits::MAX_PADS)
        .enumerate()
    {
        let pad_id = format!("pad.{}", index + 1);
        let port = gamepad.pad_ports.get(index);
        // Assigned player slot, if any. Mapping rows below scope to
        // this pad's own player: other players' rows carry defaults
        // this pad can never drive, and edits there land overrides
        // that gameplay parks, so they only mislead.
        let slot = port.and_then(|pad| pad.slot);
        // Assignment readout: the player this pad drives plus the pad's
        // own name. No drill-down: the Devices gear is the single path
        // into pad screens, so the graph stays a tree and the Kotlin
        // breadcrumb walk needs no cycle defense. Back returns.
        let slot_row = match slot.filter(|slot| PLAYER_PORTS.get(*slot).is_some()) {
            Some(slot) => info_row(
                &format!("{pad_id}.slot"),
                player_label(slot, i18n),
                String::new(),
                info.name.clone(),
            ),
            // Unassigned pads name the same vocabulary with no player:
            // the state, not the category, is what changed.
            None => info_row(
                &format!("{pad_id}.slot"),
                i18n.text(UiText::GamepadDevice),
                String::new(),
                i18n.text(UiText::NotAssigned),
            ),
        };
        // Only the hat stays manual (sticks and triggers follow the live
        // console automatically): off parks the hat while stick driving
        // continues through substitutes.
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
                &format!("{pad_id}.dpad"),
                i18n.text(UiText::DpadSource),
                i18n.text(UiText::DescDpadSource),
                i18n,
                port.is_none_or(|pad| pad.dpad_enabled),
            ),
        ];
        // Unassigned pads park every press, so mapping rows would
        // only collect latent overrides: point at Devices instead.
        // Assigned pads list only their own player's controls (all
        // systems); other players' attachments never fire from here.
        if let Some(slot) = slot {
            for input in vm.inputs() {
                rows.extend(pad_system_rows(
                    input, snapshot, &pad_id, index, slot, &reason, i18n,
                ));
            }
        } else {
            rows.push(info_row(
                &format!("{pad_id}.unassigned"),
                i18n.text(UiText::KeyMapping),
                i18n.text(UiText::DescPadUnassigned),
                String::new(),
            ));
        }
        rows.push(button_row(
            &format!("reset.{pad_id}"),
            i18n.text(UiText::ResetScreen),
            i18n.text(UiText::DescPadReset),
        ));
        out.push(SectionWire {
            id: pad_id.clone(),
            label: info.name.clone(),
            sub: i18n.text(UiText::Gamepads),
            rows,
            visible: false,
        });
    }
    out
}

/// One machine's mapping block for one pad screen: machine header plus
/// the pad's own capture rows (attachment sub-headers drop when single),
/// plus that machine's conflict rows. Empty when the pad owns nothing here.
fn pad_system_rows(
    input: &nerust_gui_viewmodel::settings::InputSettingsViewModel,
    snapshot: &SettingsSnapshot,
    pad_id: &str,
    index: usize,
    slot: usize,
    reason: &str,
    i18n: I18n,
) -> Vec<RowWire> {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    let view = input.view.get();
    let system = view.system_id.to_string();
    let mut system_chunks: Vec<(String, Vec<RowWire>)> = Vec::new();
    for section in &view.sections {
        // Collect the section's targets first so values resolve under one
        // borrow; rows render from the zipped results below.
        let mut pending: Vec<(
            CaptureTarget,
            Option<nerust_input_traits::StandardButton>,
            String,
            String,
            String,
        )> = Vec::new();
        for binding in &section.rows {
            // Per-pad mapping mirrors the game rows: same
            // attachment/control, routed to this pad. The physical
            // hat stays fixed (its switch lives on the sources
            // sub-screen), so dpad rows never render here: they would
            // only alias the hat the sources screen already gates.
            if matches!(
                binding.pad_default,
                Some(
                    nerust_input_traits::StandardButton::DpadUp
                        | nerust_input_traits::StandardButton::DpadDown
                        | nerust_input_traits::StandardButton::DpadLeft
                        | nerust_input_traits::StandardButton::DpadRight
                )
            ) {
                continue;
            }
            let (Some(attachment), Some(control)) =
                (binding.target.attachment(), binding.target.control())
            else {
                continue;
            };
            // Scope to this pad's own player: the viewmodel resolves
            // the port once per row, so unclassifiable attachments
            // (port `None`) fail open and never silently drop rows.
            if binding.port.is_some_and(|port| port != slot) {
                continue;
            }
            pending.push((
                CaptureTarget::PadBinding {
                    system: view.system_id.clone_box(),
                    pad: index,
                    attachment: attachment.to_string(),
                    control: control.to_string(),
                },
                binding.pad_default,
                attachment.to_string(),
                control.to_string(),
                binding.label.clone(),
            ));
        }
        let values = input.pad_row_values(
            &pending
                .iter()
                .map(|(target, default, _, _, _)| (target.clone(), *default))
                .collect::<Vec<_>>(),
        );
        let mut section_rows = Vec::new();
        for ((target, _, attachment, control, label), value) in pending.into_iter().zip(values) {
            let text = match &value {
                BindingValueView::Unbound(label)
                | BindingValueView::Bound(label)
                | BindingValueView::Capturing(label) => label.clone(),
            };
            section_rows.push(capture_row(
                &format!("{pad_id}.{system}.{attachment}.{control}",),
                label,
                String::new(),
                text,
                &encode_target(&target, &system),
                live,
                reason,
            ));
        }
        if section_rows.is_empty() {
            continue;
        }
        // One attachment per machine here (rows scope to this pad's own
        // slot), so its header names nothing new: drop it and let the
        // machine header carry the block. Multi-attachment machines keep
        // their headers.
        system_chunks.push((section.label.clone(), section_rows));
    }
    if system_chunks.is_empty() {
        return Vec::new();
    }
    let mut out = vec![header_row(view.label.clone())];
    let single = system_chunks.len() == 1;
    for (label, mut chunk) in system_chunks {
        if !single {
            out.push(header_row(label));
        }
        out.append(&mut chunk);
    }
    // One button driving two controls never fires the second:
    // surface it on the pad's own screen, next to the rows.
    // Scoped like the rows above: hidden attachments have no
    // rows to disambiguate against.
    if let Some(settings) = snapshot.shared.input.systems.get(&view.system_id)
        && let Some(entries) = settings.pad_bindings.get(index)
    {
        out.extend(conflict_rows(pad_id, &system, slot, entries, i18n));
    }
    out
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
        // Raw persisted bindings carry string attachments (no DTO port
        // here), so scope through the canonical traits rule: unknown
        // attachments fail open and never silently drop rows.
        let shown: Vec<_> = controls
            .iter()
            .filter(|(attachment, _)| {
                !port_index_for_slot(attachment.as_str()).is_some_and(|port| port != slot)
            })
            .collect();
        if shown.len() < 2 {
            continue;
        }
        let name = nerust_gui_viewmodel::settings::labels::pad_button_label(i18n, button);
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
