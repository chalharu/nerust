//! Per-pad settings screens: one `pad.<n>` screen per settings rank.
//!
//! Pure projection like the rest of `settings_views`: reads the view model,
//! the draft snapshot, and the vault device truth, and returns wire rows.
//! No mutation.
//!
//! Screens iterate stable settings ranks, never the live list position:
//! a disconnect/reconnect cycle compacts the live list but ranks (and the
//! session commit path) stand still, so a rank-skewed screen would edit a
//! stranger pad's overrides while the game reads another. Live-unknown
//! pads (scan raced ahead of the connect event) get info-only provisional
//! screens; they re-resolve on reopen once the mirror catches up.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{editor::CaptureTarget, i18n::UiText, model::input::DEADZONE_STEPS};

use super::wire::{
    GamepadInfo, PLAYER_PORTS, RowWire, SectionWire, button_row, capture_row, encode_target,
    header_row, info_row, live_for_rank, number_row, player_label, toggle_row,
};
use nerust_settings_core::i18n::I18n;
use nerust_settings_core::labels::pad_label;

/// Per-pad settings screens: one `pad.<n>` screen per stable settings
/// rank, with its own deadzone, hat switch, and per-system button mapping.
/// Stick/trigger routing is automatic (the live console declares analog
/// controls or not), so no toggles: only the hat stays manual.
/// Ranks (not live positions) key the screens: the session commit path
/// reads overrides by rank, so the screens must write by rank too, or a
/// reconnect cycle would silently redirect edits to a stranger pad.
/// Unconnected ranks hide their capture rows (nothing to press) but keep
/// their settings, which restore on reconnect. Ports are player slots,
/// not devices: the rank table (not device ids) is what persists.
pub(super) fn build_pad_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    gamepads: &[GamepadInfo],
    pad_order: &[String],
) -> Vec<SectionWire> {
    let gamepad = &snapshot.shared.input.gamepad;
    // Stable universe: every settings row plus every mirrored rank.
    // Unknown live members (scan raced ahead of the connect event) wait
    // for the next connect instead of earning placeholder screens.
    let width = gamepad.pad_ports.len().max(pad_order.len());
    let mut out = Vec::with_capacity(width);
    for rank in 0..width {
        let live = live_for_rank(gamepads, pad_order, rank);
        out.push(pad_rank_section(
            vm,
            snapshot,
            i18n,
            rank,
            live.map(|info| info.name.clone()),
        ));
    }
    out
}

/// One rank's screen: assignment readout, deadzone, hat switch, mapping
/// rows (assigned live ranks only), and the screen reset. Disconnected
/// ranks hide their mapping captures (nothing to press); their edits
/// persist and restore on reconnect.
fn pad_rank_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    rank: usize,
    live_name: Option<String>,
) -> SectionWire {
    // Disconnected ranks keep every row except mapping captures (nothing
    // to press); their edits persist and restore on reconnect.
    let live = live_name.is_some();
    let gamepad = &snapshot.shared.input.gamepad;
    let pad_id = format!("pad.{}", rank + 1);
    let port = gamepad.pad_ports.get(rank);
    // Assigned player slot, if any. Mapping rows below scope to
    // this pad's own player: other players' rows carry defaults
    // this pad can never drive, and edits there land overrides
    // that gameplay parks, so they only mislead.
    let slot = port.and_then(|pad| pad.slot);
    let name = live_name.unwrap_or_else(|| pad_label(i18n, rank));
    // Assignment readout: the player this pad drives plus the pad's
    // own name. No drill-down: the Devices gear is the single path
    // into pad screens, so the graph stays a tree and the Kotlin
    // breadcrumb walk needs no cycle defense. Back returns.
    let slot_row = match slot.filter(|slot| PLAYER_PORTS.get(*slot).is_some()) {
        Some(slot) => info_row(
            &format!("{pad_id}.slot"),
            player_label(slot, i18n),
            String::new(),
            name.clone(),
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
            "%".to_string(),
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
    // Disconnected ranks hide their captures (nothing to press);
    // the rows return with the pad.
    if let Some(slot) = slot.filter(|_| live) {
        for input in vm.inputs() {
            rows.extend(pad_system_rows(input, &pad_id, rank, slot));
        }
    } else if slot.is_none() {
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
    SectionWire {
        id: pad_id.clone(),
        label: name,
        sub: i18n.text(UiText::Gamepads),
        rows,
        visible: false,
    }
}

/// One machine's mapping block for one pad screen: machine header plus
/// the pad's own capture rows (attachment sub-headers drop when single).
/// Empty when the pad owns nothing here.
fn pad_system_rows(
    input: &nerust_gui_viewmodel::settings::InputSettingsViewModel,
    pad_id: &str,
    rank: usize,
    slot: usize,
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
                    pad: rank,
                    attachment: attachment.to_string(),
                    control: control.to_string(),
                },
                binding.pad_default,
                binding.label.clone(),
            ));
        }
        let values = input.pad_row_values(
            &pending
                .iter()
                .map(|(target, default, _)| (target.clone(), *default))
                .collect::<Vec<_>>(),
        );
        let mut section_rows = Vec::new();
        for ((target, _, label), value) in pending.into_iter().zip(values) {
            let text = match &value {
                BindingValueView::Unbound(label)
                | BindingValueView::Bound(label)
                | BindingValueView::Capturing(label) => label.clone(),
            };
            let (Some(attachment), Some(control)) = (target.attachment(), target.control()) else {
                continue;
            };
            section_rows.push(capture_row(
                &format!("{pad_id}.{system}.{attachment}.{control}",),
                label,
                String::new(),
                text,
                &encode_target(&target),
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
    // Conflicts surface through the payload issues (save-blocking
    // validation), never as duplicate rows: one surfacing, not two.
    out
}
