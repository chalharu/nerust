//! Player, device-picker, device-detail, and key-mapping screens.
//!
//! Pure projection like the rest of `settings_views`: reads the view model,
//! the draft snapshot, and the vault device truth, and returns wire rows.
//! No mutation.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_input_traits::{BUILTIN_KEYBOARD_MEMBER, member_is_gamepad, member_is_shareable};
use nerust_settings_core::i18n::UiText;

use super::wire::{
    GamepadInfo, KeyboardInfo, PLAYER_PORTS, RowWire, SectionWire, bare_member, binding_key,
    button_row, capture_open, capture_row, choice_row, devices_id, encode_target, group_row,
    header_row, info_row, live_for_rank, player_id, player_label, toggle_row, types_id,
};
use nerust_settings_core::i18n::I18n;

/// Player screens (one per global port): the Devices row opens the device
/// picker, the Controller-type row opens the per-machine type list.
/// Summaries read from the port device set plus pad-port assignments,
/// never from machine sections.
pub(super) fn build_player_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    pad_order: &[String],
) -> Vec<SectionWire> {
    let mut sections = Vec::new();
    for (index, port) in PLAYER_PORTS.iter().enumerate() {
        let player_id = player_id(index);
        let devices_id = devices_id(port);
        let types_id = types_id(index);
        let label = player_label(index, i18n);
        let names = port_member_names(port, snapshot, keyboards, gamepads, pad_order, i18n);
        sections.push(SectionWire {
            id: player_id.clone(),
            label: label.clone(),
            sub: i18n.text(UiText::SectionControllers),
            rows: vec![
                {
                    let mut row = group_row(
                        &format!("{player_id}.devices"),
                        i18n.text(UiText::Devices),
                        String::new(),
                        &devices_id,
                    );
                    row.text = nerust_settings_core::labels::port_summary(i18n, &names);
                    row
                },
                group_row(
                    &format!("{player_id}.type"),
                    i18n.text(UiText::ControllerType),
                    String::new(),
                    &types_id,
                ),
            ],
            visible: false,
        });
        // Controller-type list: one group row per machine that actually
        // has a slot on this port (NES-only for player 2). Ports arrive
        // resolved in the viewmodel DTO; the frontend never parses ids.
        let type_rows: Vec<RowWire> = vm
            .inputs()
            .iter()
            .filter(|input| {
                input
                    .view
                    .get()
                    .slots
                    .iter()
                    .any(|slot| slot.port == Some(index))
            })
            .map(|input| {
                let view = input.view.get();
                let summary = view
                    .slots
                    .iter()
                    .find(|slot| slot.port == Some(index))
                    .and_then(|slot| {
                        slot.selected_profile_id.as_ref().and_then(|selected| {
                            slot.choices.iter().find_map(|choice| {
                                (choice.value.as_deref() == Some(selected.as_str()))
                                    .then(|| choice.label.clone())
                            })
                        })
                    })
                    .unwrap_or_else(|| i18n.text(UiText::NotAssigned));
                group_row(
                    &format!("{types_id}.{}", view.system_id),
                    view.label.clone(),
                    summary,
                    &format!("input.{}", view.system_id),
                )
            })
            .collect();
        sections.push(SectionWire {
            id: types_id.clone(),
            label: i18n.text(UiText::ControllerType),
            sub: label.clone(),
            rows: type_rows,
            visible: false,
        });
        sections.push(build_devices_section(
            port,
            &devices_id,
            &label,
            snapshot,
            keyboards,
            gamepads,
            pad_order,
            i18n,
        ));
    }
    sections.extend(build_device_sections(vm, keyboards, i18n));
    sections
}

/// Display names for a port in set order: keyboards from the port device
/// set plus gamepads from the pad-port table. Unknown members fall back
/// to a generic label so a stale id never renders blank. Gamepad names
/// resolve by connection-order index (Pad N = Nth connected pad); an
/// assigned but disconnected pad keeps its `Pad N` label so the
/// assignment stays visible.
pub(super) fn port_member_names(
    port: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    pad_order: &[String],
    i18n: I18n,
) -> Vec<String> {
    // Effective members only: rows render solely while the terminal
    // enumerates them, so every screen shares one enumeration truth.
    // Otherwise a reconnectable USB keyboard would read as assigned on
    // the player screens while the device list reports none connected.
    // Unplugged USB descriptors stay assigned (reconnect restores them).
    let builtin_present = keyboards
        .iter()
        .any(|info| info.member == BUILTIN_KEYBOARD_MEMBER);
    let mut names: Vec<String> = snapshot
        .app_state
        .port_devices
        .get(port)
        .map(|members| {
            members
                .iter()
                .filter(|member| {
                    if member.as_str() == BUILTIN_KEYBOARD_MEMBER {
                        builtin_present
                    } else if member_is_shareable(member) {
                        keyboards.iter().any(|info| info.member == **member)
                    } else {
                        true
                    }
                })
                .map(|member| keyboard_member_name(member, keyboards, i18n))
                .collect()
        })
        .unwrap_or_default();
    let port_index = PLAYER_PORTS.iter().position(|known| *known == port);
    if let Some(port_index) = port_index {
        // Rank-indexed like the session commit path: the live list
        // compacts on disconnect, so a live position would name the
        // wrong pad after a reconnect cycle.
        for (rank, pad) in snapshot.shared.input.gamepad.pad_ports.iter().enumerate() {
            if pad.slot == Some(port_index)
                && let Some(info) = live_for_rank(gamepads, pad_order, rank)
            {
                names.push(info.name.clone());
            }
        }
    }
    names
}

/// Reserved slot holding a disconnected pad: restores on reconnect, so
/// the radio ("no pad assigned") never lies about it. Shared by
/// the empty and connected branches below.
fn stale_slot(port_index: Option<usize>, snapshot: &SettingsSnapshot) -> Option<usize> {
    port_index.and_then(|port_index| {
        snapshot
            .shared
            .input
            .gamepad
            .pad_ports
            .iter()
            .position(|pad| pad.slot == Some(port_index))
    })
}

/// Drop the built-in membership when fresh enumeration reports no
/// built-in keyboard: a capability flag, not a known device, so there is
/// nothing to restore on reconnect. USB descriptors are never pruned
/// here — unplug/replug cycles keep their assignment.
pub(super) fn prune_missing_builtin(snapshot: &mut SettingsSnapshot, keyboards: &[KeyboardInfo]) {
    // No evidence, no pruning: an empty scan (transient enumeration race)
    // must never wipe persisted membership. A non-empty scan missing the
    // built-in row is positive absence and prunes.
    if keyboards.is_empty() {
        return;
    }
    if keyboards
        .iter()
        .any(|info| info.member == BUILTIN_KEYBOARD_MEMBER)
    {
        return;
    }
    for members in snapshot.app_state.port_devices.values_mut() {
        members.retain(|member| member != BUILTIN_KEYBOARD_MEMBER);
    }
}

pub(super) fn keyboard_member_name(member: &str, keyboards: &[KeyboardInfo], i18n: I18n) -> String {
    if member == BUILTIN_KEYBOARD_MEMBER {
        i18n.text(UiText::BuiltInKeyboard)
    } else if let Some(info) = keyboards.iter().find(|info| info.member == member) {
        if info.name.trim().is_empty() {
            // OS gave no name: fall back instead of rendering blank.
            i18n.text(UiText::ExternalKeyboard)
        } else {
            info.name.clone()
        }
    } else if member_is_shareable(member) {
        i18n.text(UiText::ExternalKeyboard)
    } else if member_is_gamepad(member) {
        i18n.text(UiText::GamepadDevice)
    } else {
        member.to_string()
    }
}

/// Device picker: the dynamic keyboard enumeration (built-in row when
/// present, one row per external descriptor, BT/USB agnostic, virtual
/// devices excluded — classified Kotlin-side). Row tap toggles port
/// membership; Details opens the device screen. Gamepads are radio
/// assigned here: at most one connected pad drives a player, Unassigned
/// parks them all. The choice edits the same pad-port table as Controls
/// > Gamepads (deadzone and hat switch stay there).
#[allow(
    clippy::too_many_arguments,
    reason = "Projection seam: truth (snapshot, lists, order, locale) threads through"
)]
pub(super) fn build_devices_section(
    port: &str,
    devices_id: &str,
    player: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
    pad_order: &[String],
    i18n: I18n,
) -> SectionWire {
    let mut rows = keyboard_rows(port, devices_id, snapshot, keyboards, i18n);
    rows.extend(gamepad_rows(
        port, devices_id, snapshot, gamepads, pad_order, i18n,
    ));
    rows.push(button_row(
        &format!("reset.{devices_id}"),
        i18n.text(UiText::ResetScreen),
        String::new(),
    ));
    SectionWire {
        id: devices_id.to_string(),
        label: i18n.text(UiText::Devices),
        sub: player.to_string(),
        rows,
        visible: false,
    }
}

/// Keyboard membership toggles for one port: the dynamic enumeration
/// (built-in row when present, one row per external descriptor). Row tap
/// toggles port membership; Details opens the device screen.
fn keyboard_rows(
    port: &str,
    devices_id: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    i18n: I18n,
) -> Vec<RowWire> {
    let members = snapshot.app_state.port_devices.get(port);
    let is_member = |member: &str| {
        members
            .map(|set| set.iter().any(|entry| entry == member))
            .unwrap_or(false)
    };
    let mut rows = vec![header_row(i18n.text(UiText::Keyboard))];
    if keyboards.is_empty() {
        rows.push(info_row(
            &format!("{devices_id}.none"),
            i18n.text(UiText::NoKeyboards),
            i18n.text(UiText::DescNoKeyboards),
            String::new(),
        ));
    }
    for info in keyboards {
        let mut row = toggle_row(
            &format!("devices.{port}.{}", info.member),
            keyboard_member_name(&info.member, keyboards, i18n),
            if info.builtin {
                i18n.text(UiText::AlwaysAvailable)
            } else {
                i18n.text(UiText::Connected)
            },
            i18n,
            is_member(&info.member),
        );
        // The opaque member id rides the target; the port comes from the key.
        row.target = info.member.clone();
        row.detail_target = format!("device.{}", info.member);
        rows.push(row);
    }
    rows
}

/// Gamepad radio, stale reservation, and gears for one port. At most one
/// connected pad drives a player; "no pad" parks them all. The choice
/// edits the same pad-port table as Controls > Gamepads (deadzone and
/// hat switch stay there).
fn gamepad_rows(
    port: &str,
    devices_id: &str,
    snapshot: &SettingsSnapshot,
    gamepads: &[GamepadInfo],
    pad_order: &[String],
    i18n: I18n,
) -> Vec<RowWire> {
    let mut rows = Vec::new();
    let port_index = PLAYER_PORTS.iter().position(|known| *known == port);
    // No connected pads: no header, no dead row. The radio below names
    // pads, so with none connected there is nothing to pick or gear into;
    // only a stale slot reservation (restores on reconnect) still renders.
    // Name pad support explicitly so the screen never reads as
    // keyboard-only with no pads attached.
    if gamepads.is_empty() {
        rows.push(header_row(i18n.text(UiText::Gamepads)));
        rows.push(info_row(
            &format!("{devices_id}.gamepad.none"),
            i18n.text(UiText::GamepadDevice),
            i18n.text(UiText::DescNoGamepads),
            String::new(),
        ));
        // Stale reservations restore silently on reconnect; the none row
        // above already says no pad drives this player right now.
        return rows;
    }
    rows.push(header_row(i18n.text(UiText::Gamepads)));
    // Radio over the connected pads, valued by stable rank: one player,
    // one pad. The none choice parks every pad for this player, so it
    // never reads as "this port has none" the way a disconnected state
    // does. Values are ranks (the session commit index), never live
    // positions, so a reconnect cycle cannot redirect the assignment.
    let mut pad_choices: Vec<(Option<usize>, String)> =
        vec![(None, i18n.text(UiText::NotAssigned))];
    for (rank, descriptor) in pad_order.iter().enumerate() {
        if let Some(info) = gamepads
            .iter()
            .find(|info| bare_member(&info.member) == *descriptor)
        {
            pad_choices.push((Some(rank), info.name.clone()));
        }
    }
    let assigned: Option<usize> = stale_slot(port_index, snapshot)
        // Disconnected assigned pads hide: the radio only names
        // connected pads, and the stale slot restores on reconnect.
        .filter(|rank| live_for_rank(gamepads, pad_order, *rank).is_some());
    rows.push(choice_row(
        &format!("{devices_id}.gamepad"),
        i18n.text(UiText::GamepadDevice),
        i18n.text(UiText::DescGamepadRadio),
        &pad_choices,
        &assigned,
    ));
    // The reservation restores silently on reconnect; the radio already
    // reads None while its pad is away, so no second row repeats it.
    rows.extend(gear_rows(
        port_index, devices_id, snapshot, gamepads, pad_order, i18n,
    ));
    rows
}

/// Gear into this port's own pad screen (deadzone, hat switch, button
/// mapping): the radio owns assignment, the gear owns the pad.
/// Unassigned pads live under player 1 only, so the same pad never gears
/// in from two players at once.
fn gear_rows(
    port_index: Option<usize>,
    devices_id: &str,
    snapshot: &SettingsSnapshot,
    gamepads: &[GamepadInfo],
    pad_order: &[String],
    i18n: I18n,
) -> Vec<RowWire> {
    let mut rows = Vec::new();
    // Rank-indexed like the radio and the session commit path: the live
    // list compacts on disconnect, so live positions would gear into a
    // stranger pad's screen after a reconnect cycle.
    for (rank, descriptor) in pad_order.iter().enumerate() {
        let entry_slot: Option<usize> = snapshot
            .shared
            .input
            .gamepad
            .pad_ports
            .get(rank)
            .and_then(|pad| pad.slot);
        let assigned_here = entry_slot.is_some_and(|slot| Some(slot) == port_index)
            || (entry_slot.is_none() && port_index == Some(0));
        if !assigned_here {
            continue;
        }
        let info = gamepads
            .iter()
            .find(|info| bare_member(&info.member) == *descriptor);
        let Some(info) = info else {
            continue;
        };
        // Unassigned pads stay reachable from player 1: say so on the
        // gear itself, so this screen never promises another
        // player's pad.
        let gear_description = if entry_slot.is_none() {
            i18n.text(UiText::NotAssigned)
        } else {
            i18n.text(UiText::DescGamepadGear)
        };
        rows.push(group_row(
            &format!("{devices_id}.pad{}", rank + 1),
            info.name.clone(),
            gear_description,
            &format!("pad.{}", rank + 1),
        ));
    }
    rows
}

/// One device screen per enumerated keyboard: the per-system
/// key-mapping index (shared single table, not per device).
pub(super) fn build_device_sections(
    vm: &SettingsViewModel,
    keyboards: &[KeyboardInfo],
    i18n: I18n,
) -> Vec<SectionWire> {
    keyboards
        .iter()
        .map(|info| {
            let mut rows = vec![header_row(i18n.text(UiText::KeyMapping))];
            for system in vm.systems() {
                let sid = system.system_id().to_string();
                rows.push(group_row(
                    &format!("device.{}.keys.{sid}", info.member),
                    system.display_name().to_string(),
                    String::new(),
                    &format!("keys.{sid}"),
                ));
            }
            SectionWire {
                id: format!("device.{}", info.member),
                label: if info.builtin {
                    i18n.text(UiText::BuiltInKeyboard)
                } else if info.name.trim().is_empty() {
                    i18n.text(UiText::ExternalKeyboard)
                } else {
                    info.name.clone()
                },
                sub: i18n.text(UiText::Devices),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// One key-mapping screen per machine. Reached from each keyboard device
/// detail screen; the controller screens hold slots only. Capture takes
/// any mapped hardware key; with no input device the capture rows hide
/// and one shared empty state names the missing input (shortcuts policy).
pub(super) fn build_keys_sections(
    vm: &SettingsViewModel,
    i18n: I18n,
    keyboards: &[KeyboardInfo],
    gamepads: &[GamepadInfo],
) -> Vec<SectionWire> {
    let open = capture_open(keyboards, gamepads);
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            let mut rows: Vec<RowWire> = Vec::new();
            if !open {
                rows.push(info_row(
                    &format!("keys.{}.none", view.system_id),
                    i18n.text(UiText::NoInputDevices),
                    i18n.text(UiText::DescNoInputDevices),
                    String::new(),
                ));
            }
            for section in &view.sections {
                let mut section_rows: Vec<RowWire> = Vec::new();
                if open {
                    section_rows.extend(section.rows.iter().map(|binding| {
                        capture_row(
                            &format!(
                                "keys.{}.binding.{}",
                                view.system_id,
                                binding_key(&binding.target)
                            ),
                            binding.label.clone(),
                            String::new(),
                            binding_value_text(binding),
                            &encode_target(&binding.target),
                        )
                    }));
                }
                if section_rows.is_empty() {
                    continue;
                }
                rows.push(header_row(section.label.clone()));
                rows.extend(section_rows);
            }
            rows.push(button_row(
                &format!("reset.keys.{}", view.system_id),
                i18n.text(UiText::ResetScreen),
                String::new(),
            ));
            SectionWire {
                id: format!("keys.{}", view.system_id),
                label: view.label.clone(),
                sub: i18n.text(UiText::KeyMapping),
                rows,
                visible: false,
            }
        })
        .collect()
}

pub(super) fn binding_value_text(
    binding: &nerust_gui_viewmodel::settings::dto::BindingRowView,
) -> String {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    match &binding.value {
        BindingValueView::Unbound(label)
        | BindingValueView::Bound(label)
        | BindingValueView::Capturing(label) => label.clone(),
    }
}
