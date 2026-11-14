//! Android settings commands: one wire op applied to the draft snapshot.
//!
//! Each op mutates through exactly one path (view model, Android-local
//! fields, device membership, or wholesale reset), then the caller projects
//! once. At most one view-model build per command.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, language_from_locale_tag},
    model::{
        app_state::DesktopAppState,
        input::ensure_default_shortcuts,
        language::AppLanguage,
        local::{
            HostBackendLocalSettings, NotificationAreaMode, NotificationColor, NotificationPosition,
        },
        shared::{DesktopSharedSettings, StoragePolicy},
    },
};
use nerust_system_traits::factory::descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId};

use super::projection::{
    PLAYER_PORTS, build_sections, project_fresh, project_payload, prune_missing_builtin,
};
use super::wire::{
    CaptureSlot, GamepadInfo, I18n, KeyboardInfo, Projection, RowKind, RowWire, SectionWire,
    ViewPayload, VmDeps, WireCommand, WireOp, build_vm, choice, find_row, pin_draft_language,
    projection, range_value, resolve_display, sync_snapshot_from_vm, toggle_value, with_vm_deps,
};

/// Apply one wire command to the draft snapshot in place, returning the
/// refreshed payload. Stateless per call except for the passed draft,
/// capture slot, vault keyboard list, touch capability, counted gamepad
/// list, and terminal locale. Exactly one view-model build per command:
/// snapshot-direct ops (device truth, reset, capture cancel) project fresh
/// afterwards, while view-model ops reuse the same model for apply and
/// project instead of rebuilding per op.
pub(crate) fn apply_command_to(
    snapshot: &mut SettingsSnapshot,
    command_json: &str,
    capture: &mut CaptureSlot,
    keyboards: &mut Vec<KeyboardInfo>,
    touch_capable: &mut Option<bool>,
    gamepads: &mut Vec<GamepadInfo>,
    system_locale: &mut Option<AppLanguage>,
) -> Result<ViewPayload, String> {
    let command: WireCommand =
        serde_json::from_str(command_json).map_err(|error| error.to_string())?;
    with_vm_deps(|deps| {
        let i18n = resolve_display(snapshot.shared.general.language, *system_locale);
        let truth = projection(keyboards, *touch_capable, gamepads, i18n);
        match command.op {
            WireOp::DeviceTruth => {
                apply_device_truth_command(
                    keyboards,
                    touch_capable,
                    gamepads,
                    system_locale,
                    &command,
                )?;
                prune_missing_builtin(snapshot, keyboards);
                project_after_snapshot_edit(
                    snapshot,
                    deps,
                    capture,
                    keyboards,
                    *touch_capable,
                    gamepads,
                    *system_locale,
                )
            }
            WireOp::ResetSection => {
                apply_reset_section(snapshot, deps, &command, truth)?;
                project_after_snapshot_edit(
                    snapshot,
                    deps,
                    capture,
                    keyboards,
                    *touch_capable,
                    gamepads,
                    *system_locale,
                )
            }
            WireOp::CancelCapture => {
                *capture = None;
                project_after_snapshot_edit(
                    snapshot,
                    deps,
                    capture,
                    keyboards,
                    *touch_capable,
                    gamepads,
                    *system_locale,
                )
            }
            WireOp::ClearBinding | WireOp::StartCapture => {
                let vm = build_vm(snapshot, deps, i18n)?;
                let sections = build_sections(&vm, snapshot, truth);
                apply_capture_command(snapshot, deps, &vm, &sections, &command, capture, truth)?;
                // Capture ops mutate through this same model (or not at
                // all), so it is current: project from it, no rebuild.
                Ok(project_payload(&vm, snapshot, deps, capture, truth))
            }
            WireOp::Set => {
                let vm = build_vm(snapshot, deps, i18n)?;
                let sections = build_sections(&vm, snapshot, truth);
                if apply_set_command(snapshot, deps, &vm, &sections, &command, i18n)? {
                    // The view model holds the mutation: project from it.
                    Ok(project_payload(&vm, snapshot, deps, capture, truth))
                } else {
                    // Snapshot-direct mutation (language, Android-local,
                    // device membership): re-resolve and project fresh.
                    project_after_snapshot_edit(
                        snapshot,
                        deps,
                        capture,
                        keyboards,
                        *touch_capable,
                        gamepads,
                        *system_locale,
                    )
                }
            }
        }
    })
}

/// Re-resolve the display language (the command may have changed it) and
/// project exactly once. Snapshot-direct ops funnel here.
fn project_after_snapshot_edit(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    capture: &CaptureSlot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
    gamepads: &[GamepadInfo],
    system_locale: Option<AppLanguage>,
) -> Result<ViewPayload, String> {
    let i18n = resolve_display(snapshot.shared.general.language, system_locale);
    let truth = projection(keyboards, touch_capable, gamepads, i18n);
    project_fresh(snapshot, deps, capture, truth)
}

/// One `set` edit: route by key family (Android-local overlay/cartridge
/// keys, port device membership, everything else through the view model).
/// Returns whether the view model holds the mutation (project from it) or
/// the snapshot was written directly (project fresh).
fn apply_set_command(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    vm: &SettingsViewModel,
    sections: &[SectionWire],
    command: &WireCommand,
    i18n: I18n,
) -> Result<bool, String> {
    // Language bypasses the VM round-trip: the VM works on the concretized
    // display copy, so an explicit choice equal to the resolved locale
    // (SystemDefault under `ja` system + explicit Japanese) reads back as
    // "untouched" and the pin-back in `sync_snapshot_from_vm` silently
    // discards it. Writing the draft directly keeps the explicit choice.
    if command.key.as_str() == "general.language" {
        let row = find_row(sections, &command.key)?;
        snapshot.shared.general.language = choice(row, command.index)?;
        return Ok(false);
    }
    if apply_android_local(snapshot, sections, command)? {
        return Ok(false);
    }
    if is_gamepad_radio_key(&command.key) {
        apply_gamepad_radio_command(snapshot, sections, command)?;
        return Ok(false);
    }
    if is_device_key(&command.key) {
        apply_device_command(snapshot, sections, command)?;
        Ok(false)
    } else {
        apply_view_model_command(vm, deps, sections, command)?;
        sync_snapshot_from_vm(snapshot, vm, i18n);
        Ok(true)
    }
}

/// Capture ops: start/clear against the vault slot. Binding targets decode
/// from the row's opaque target; the row key only locates the row. Clears
/// mutate the draft through the VM snapshot round-trip, exactly like every
/// other setter. The caller builds the view model and sections once and
/// reuses them for projection.
fn apply_capture_command(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    vm: &SettingsViewModel,
    sections: &[SectionWire],
    command: &WireCommand,
    capture: &mut CaptureSlot,
    truth: Projection,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
    if row.kind != RowKind::Capture {
        return Err(format!("{}: not a capture row", command.key));
    }
    let target = decode_target(&row.target, deps)?;
    match command.op {
        WireOp::ClearBinding => {
            // Target first, column second: game and per-pad mapping rows
            // clear whole (they never carry a pad twin); per-device
            // shortcut rows clear only their own column so a keyboard
            // Clear never strands a pad twin (and vice versa). The column
            // rides the row key suffix (`.kbd` vs `.padN`).
            match (&target, clear_column_for_key(&command.key)) {
                (CaptureTarget::Shortcut(_), Column::Key) => {
                    vm.capture
                        .clear_key(&target)
                        .map_err(|error| error.to_string())?;
                }
                (CaptureTarget::Shortcut(_), Column::Pad) => {
                    vm.capture
                        .clear_pad(&target)
                        .map_err(|error| error.to_string())?;
                }
                _ => {
                    vm.capture
                        .clear_binding(&target)
                        .map_err(|error| error.to_string())?;
                }
            }
            sync_snapshot_from_vm(snapshot, vm, truth.i18n);
            *capture = None;
            Ok(())
        }
        WireOp::StartCapture => {
            // The disabled row carries the reason: reject only via the
            // row's enabled state, never via a second truth check here,
            // so rows and the command path cannot disagree.
            if !row.enabled {
                return Err(truth.i18n.text(UiText::InputDeviceRequired));
            }
            vm.capture
                .start_capture(target.clone())
                .map_err(|error| error.to_string())?;
            let prompt = vm.capture.view.get().prompt.clone();
            *capture = Some((target, prompt));
            Ok(())
        }
        op => Err(format!("unknown capture op: {op:?}")),
    }
}

/// Which shortcut column a clear addresses, from the row key suffix
/// (`.kbd` vs `.padN`). Game and per-pad mapping rows carry no suffix
/// and clear whole.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    Key,
    Pad,
    Whole,
}

fn clear_column_for_key(key: &str) -> Column {
    if key.ends_with(".kbd") {
        Column::Key
    } else if key.contains(".pad") {
        Column::Pad
    } else {
        Column::Whole
    }
}

/// Required JSON array field of the device-truth payload.
fn payload_vec<T>(command: &WireCommand, field: &str) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let raw = command
        .payload
        .get(field)
        .ok_or_else(|| format!("device_truth op misses {field}"))?;
    serde_json::from_value(raw.clone()).map_err(|error| error.to_string())
}

/// Single device-truth push (Kotlin `InputDevice` enumeration, hotplug
/// refresh included): keyboards, touch capability, counted gamepads, and
/// the terminal locale in one op, one re-projection. Ephemeral truth,
/// never persisted; the draft is untouched. `lang` is optional (unknown
/// terminal locale keeps the previous push); the rest fail loud.
fn apply_device_truth_command(
    keyboards: &mut Vec<KeyboardInfo>,
    touch_capable: &mut Option<bool>,
    gamepads: &mut Vec<GamepadInfo>,
    system_locale: &mut Option<AppLanguage>,
    command: &WireCommand,
) -> Result<(), String> {
    let list: Vec<KeyboardInfo> = payload_vec(command, "keyboards")?;
    for info in &list {
        if info.member.is_empty() {
            return Err("keyboard member id must not be empty".to_string());
        }
    }
    *keyboards = list;
    let capable_raw = command
        .payload
        .get("capable")
        .ok_or_else(|| "device_truth op misses capable".to_string())?;
    *touch_capable =
        Some(serde_json::from_value(capable_raw.clone()).map_err(|error| error.to_string())?);
    let names: Vec<GamepadInfo> = payload_vec(command, "gamepads")?;
    for info in &names {
        if info.member.is_empty() {
            return Err("gamepad member id must not be empty".to_string());
        }
        if info.name.is_empty() {
            return Err("gamepad name must not be empty".to_string());
        }
    }
    *gamepads = names;
    if let Some(raw) = command.payload.get("lang") {
        let tag = raw
            .as_str()
            .ok_or_else(|| "device_truth lang must be a string".to_string())?;
        if tag.is_empty() {
            return Err("device_truth lang must not be empty".to_string());
        }
        *system_locale = Some(language_from_locale_tag(tag));
    }
    Ok(())
}

/// Gamepad radio rows: `devices.<port>.gamepad` choices. Values are the
/// connection-order pad indices (`None` parks every pad on this port).
/// One player drives one pad: picking a pad parks its mates on this port
/// and pulls it off any other port, so validation never trips.
fn is_gamepad_radio_key(key: &str) -> bool {
    key.strip_suffix(".gamepad").is_some_and(|head| {
        head.strip_prefix("devices.")
            .is_some_and(|port| PLAYER_PORTS.contains(&port))
    })
}

fn apply_gamepad_radio_command(
    snapshot: &mut SettingsSnapshot,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
    if row.kind != RowKind::Choice {
        return Err(format!("{}: not a gamepad radio row", command.key));
    }
    let port = command
        .key
        .split('.')
        .nth(1)
        .ok_or_else(|| format!("malformed device key: {}", command.key))?;
    let Some(port_index) = PLAYER_PORTS.iter().position(|known| *known == port) else {
        return Err(format!("unknown port: {port}"));
    };
    let pad: Option<usize> = choice(row, command.index)?;
    if let Some(pad) = pad {
        if snapshot.shared.input.gamepad.pad_ports.get(pad).is_none() {
            return Err(format!("{}: unknown pad index {pad}", command.key));
        }
        for (index, entry) in snapshot
            .shared
            .input
            .gamepad
            .pad_ports
            .iter_mut()
            .enumerate()
        {
            if index == pad {
                entry.slot = Some(port_index);
            } else if entry.slot == Some(port_index) {
                entry.slot = None;
            }
        }
    } else {
        for entry in snapshot.shared.input.gamepad.pad_ports.iter_mut() {
            if entry.slot == Some(port_index) {
                entry.slot = None;
            }
        }
    }
    Ok(())
}

/// Device-membership rows: `devices.<port>.<member>` toggles. The member
/// id rides in the row target; the port comes from the key prefix.
/// Keyboards share ports; gamepad exclusivity is a validation issue
/// (fixable by untoggling), never a silent refusal.
fn is_device_key(key: &str) -> bool {
    key.starts_with("devices.")
}

fn apply_device_command(
    snapshot: &mut SettingsSnapshot,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
    if row.kind != RowKind::Toggle {
        return Err(format!("{}: not a device row", command.key));
    }
    let member = row.target.clone();
    if member.is_empty() {
        return Err(format!("{}: device row misses member", command.key));
    }
    let port = command
        .key
        .split('.')
        .nth(1)
        .ok_or_else(|| format!("malformed device key: {}", command.key))?;
    if !PLAYER_PORTS.contains(&port) {
        return Err(format!("unknown port: {port}"));
    }
    let members = snapshot
        .app_state
        .port_devices
        .entry(port.to_string())
        .or_default();
    let enrolled = toggle_value(row, command.index)?;
    if enrolled && !members.iter().any(|member_id| member_id == &member) {
        members.push(member);
    } else if !enrolled {
        members.retain(|member_id| member_id != &member);
    }
    Ok(())
}

/// Canonical defaults for a reset: stock shared/local/app state plus each
/// factory's default system settings, with canonical shortcut bindings
/// seeded (mirrors the load-time migration).
fn default_snapshot(deps: &VmDeps) -> SettingsSnapshot {
    let mut shared = DesktopSharedSettings::default();
    for factory in &deps.factories {
        if let Some(defaults) = factory
            .as_system_defaults()
            .and_then(|system_defaults| system_defaults.default_system_settings())
        {
            shared.systems.insert(factory.system_id(), defaults);
        }
    }
    ensure_default_shortcuts(&mut shared.input.shortcuts);
    SettingsSnapshot {
        shared,
        local: HostBackendLocalSettings::default(),
        app_state: DesktopAppState::default(),
    }
}

/// Reset one screen (`reset.<section-id>`) or everything (`reset.all`) to
/// defaults. `reset.all` swaps the whole snapshot wholesale; a scoped reset
/// copies that screen's backing fields wholesale from a default snapshot,
/// then the caller projects once. No per-row replay, no probe per row.
fn apply_reset_section(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    command: &WireCommand,
    truth: Projection,
) -> Result<(), String> {
    let scope = command
        .key
        .strip_prefix("reset.")
        .ok_or_else(|| format!("malformed reset key: {}", command.key))?;
    if scope == "all" {
        // Membership rows only cover enumerated keyboards; stale members
        // have no rows, so the whole-snapshot swap restores the port map
        // wholesale along with everything else.
        *snapshot = default_snapshot(deps);
        return Ok(());
    }
    let default = default_snapshot(deps);
    apply_reset_scope(snapshot, &default, deps, scope, truth)
}

/// One screen's backing fields, copied wholesale from the default snapshot.
/// Navigation-only screens (machine links, player/type pickers, device
/// detail screens) have no editable state of their own: resetting them is
/// a no-op, exactly like the old per-row replay that skipped their rows.
fn apply_reset_scope(
    snapshot: &mut SettingsSnapshot,
    default: &SettingsSnapshot,
    deps: &VmDeps,
    scope: &str,
    truth: Projection,
) -> Result<(), String> {
    match scope {
        "play" => {
            snapshot.local.screen_orientation = default.local.screen_orientation;
            snapshot.local.audio.muted = default.local.audio.muted;
            snapshot.local.audio.master_volume_percent = default.local.audio.master_volume_percent;
        }
        "saves" => {
            snapshot.shared.persistence.storage_policy = default.shared.persistence.storage_policy;
        }
        "app" => {
            snapshot.shared.general.language = default.shared.general.language;
        }
        "advanced" => {
            snapshot.local.video.presentation.vsync = default.local.video.presentation.vsync;
            snapshot.local.cartridge_peripherals = default.local.cartridge_peripherals.clone();
        }
        "about" | "player.1" | "player.2" | "types.1" | "types.2" => {}
        "audio.detail" => {
            snapshot.local.audio.sample_rate = default.local.audio.sample_rate;
            snapshot.local.audio.latency_ms = default.local.audio.latency_ms;
        }
        "notifications" => {
            snapshot.local.notifications = default.local.notifications.clone();
        }
        "controllers" => {
            snapshot.local.touch_overlay.visibility = default.local.touch_overlay.visibility;
            snapshot.local.touch_overlay.haptics = default.local.touch_overlay.haptics;
        }
        "controls.overlay_detail" => {
            snapshot.local.touch_overlay.opacity_percent =
                default.local.touch_overlay.opacity_percent;
            snapshot.local.touch_overlay.scale_percent = default.local.touch_overlay.scale_percent;
            snapshot.local.touch_overlay.vertical_offset_percent =
                default.local.touch_overlay.vertical_offset_percent;
        }
        "controls.shortcuts" => {
            snapshot.shared.input.shortcuts = default.shared.input.shortcuts.clone();
        }
        scope => {
            return apply_reset_dynamic_scope(snapshot, default, deps, scope, truth);
        }
    }
    Ok(())
}

/// Dynamic sections: per-machine settings/inputs/keys, per-port device
/// pickers, and per-device detail screens. Unknown scopes fail loud so a
/// tampered client never no-ops silently.
fn apply_reset_dynamic_scope(
    snapshot: &mut SettingsSnapshot,
    default: &SettingsSnapshot,
    deps: &VmDeps,
    scope: &str,
    truth: Projection,
) -> Result<(), String> {
    if let Some(port) = scope
        .strip_prefix("devices.")
        .filter(|port| PLAYER_PORTS.contains(port))
    {
        // Row toggles converge on the default set; copy it (and its
        // presence) directly instead of replaying per-row diffs. The radio
        // restores this port's pad slots the same way, so one reset clears
        // both halves of the screen.
        let members = default
            .app_state
            .port_devices
            .get(port)
            .cloned()
            .unwrap_or_default();
        snapshot
            .app_state
            .port_devices
            .insert(port.to_string(), members);
        if let Some(port_index) = PLAYER_PORTS.iter().position(|known| *known == port) {
            for (index, entry) in snapshot
                .shared
                .input
                .gamepad
                .pad_ports
                .iter_mut()
                .enumerate()
            {
                let slot = default
                    .shared
                    .input
                    .gamepad
                    .pad_ports
                    .get(index)
                    .and_then(|pad| pad.slot)
                    .filter(|slot| *slot == port_index);
                if slot.is_some() || entry.slot == Some(port_index) {
                    entry.slot = slot;
                }
            }
        }
        return Ok(());
    }
    if let Some(system) = scope.strip_prefix("system.") {
        if !is_known_system(deps, system) {
            return Err(format!("unknown reset scope: {scope}"));
        }
        copy_system_entry(
            &mut snapshot.shared.systems,
            &default.shared.systems,
            system,
        );
        return Ok(());
    }
    if let Some(system) = scope.strip_prefix("input.") {
        if !is_known_system(deps, system) {
            return Err(format!("unknown reset scope: {scope}"));
        }
        copy_system_entry(
            &mut snapshot.app_state.controller_assignments,
            &default.app_state.controller_assignments,
            system,
        );
        return Ok(());
    }
    if let Some(system) = scope.strip_prefix("keys.") {
        if !is_known_system(deps, system) {
            return Err(format!("unknown reset scope: {scope}"));
        }
        copy_system_entry(
            &mut snapshot.shared.input.systems,
            &default.shared.input.systems,
            system,
        );
        return Ok(());
    }
    if let Some(member) = scope.strip_prefix("device.") {
        // Detail screens carry no editable state; the section exists only
        // while the terminal enumerates the member.
        if truth.keyboards.iter().any(|info| info.member == member) {
            return Ok(());
        }
        return Err(format!("unknown reset scope: {scope}"));
    }
    if let Some(pad) = scope.strip_prefix("pad.") {
        // One pad's own screen: deadzone, trigger doubling, and that
        // pad's per-system overrides return to defaults. The player-slot
        // assignment stays: the devices radio owns it, not this screen.
        let index: usize = pad
            .parse()
            .ok()
            .filter(|index: &usize| (1..=nerust_input_traits::MAX_PADS).contains(index))
            .ok_or_else(|| format!("unknown reset scope: {scope}"))?
            - 1;
        let Some(default_port) = default.shared.input.gamepad.pad_ports.get(index) else {
            return Err(format!("unknown reset scope: {scope}"));
        };
        let Some(port) = snapshot.shared.input.gamepad.pad_ports.get_mut(index) else {
            return Err(format!("unknown reset scope: {scope}"));
        };
        port.deadzone_percent = default_port.deadzone_percent;
        port.triggers_as_shoulders = default_port.triggers_as_shoulders;
        for settings in snapshot.shared.input.systems.values_mut() {
            if let Some(entries) = settings.pad_bindings.get_mut(index) {
                entries.clear();
            }
        }
        return Ok(());
    }
    Err(format!("unknown reset scope: {scope}"))
}

/// A factory id from the installed registry backs the per-machine screens.
fn is_known_system(deps: &VmDeps, system: &str) -> bool {
    deps.factories
        .iter()
        .any(|factory| factory.system_id().to_string() == system)
}

/// Wholesale per-system entry: the default's value, or absent when the
/// default has none (unbound slots, cleared bindings).
fn copy_system_entry<K, V>(
    map: &mut std::collections::HashMap<K, V>,
    default: &std::collections::HashMap<K, V>,
    system: &str,
) where
    K: Eq + std::hash::Hash + Clone + std::fmt::Display,
    V: Clone,
{
    let default_pair = default
        .iter()
        .find(|(key, _)| key.to_string() == system)
        .map(|(key, value)| (key.clone(), value.clone()));
    let stale: Vec<K> = map
        .keys()
        .filter(|key| key.to_string() == system)
        .cloned()
        .collect();
    for key in stale {
        map.remove(&key);
    }
    if let Some((key, value)) = default_pair {
        map.insert(key, value);
    }
}

/// Validate the draft through the view model. Conflicts and factory rules run here.
/// Factories come from the installed process-wide deps (same registry the
/// screen projects with), so there is exactly one deps path.
pub(crate) fn finish_snapshot(
    snapshot: &SettingsSnapshot,
    system_locale: Option<AppLanguage>,
) -> Result<SettingsSnapshot, Vec<String>> {
    with_vm_deps(|deps| {
        let i18n = resolve_display(snapshot.shared.general.language, system_locale);
        let vm = build_vm(snapshot, deps, i18n)?;
        vm.finish()
            .map(|mut done| {
                // The VM validated the display copy; the saved file keeps the
                // user's choice, never the resolved display value.
                pin_draft_language(
                    snapshot.shared.general.language,
                    i18n.language(),
                    &mut done.shared.general.language,
                );
                done
            })
            .map_err(|state| {
                state
                    .issues
                    .iter()
                    .map(|issue| issue.message(i18n.language()))
                    .collect()
            })
    })
    .map_err(|error| vec![error])
}

/// 1-based pad-screen segment (`pad.3.deadzone` → `2`). Rejects
/// out-of-range screens before the view-model guard, so a tampered wire
/// key fails loud here.
fn pad_screen_index(key: &str) -> Result<usize, String> {
    let index: usize = key
        .strip_prefix("pad.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|segment| segment.parse().ok())
        .ok_or_else(|| format!("bad pad key: {key}"))?;
    (1..=nerust_input_traits::MAX_PADS)
        .contains(&index)
        .then_some(index - 1)
        .ok_or_else(|| format!("bad pad key: {key}"))
}

fn apply_view_model_command(
    vm: &SettingsViewModel,
    deps: &VmDeps,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
    match command.key.as_str() {
        // Language bypasses here: `apply_set_command` writes the draft
        // directly so an explicit choice equal to the resolved locale
        // survives (the VM works on the concretized display copy).
        "general.storage_policy" => {
            let value: StoragePolicy = choice(row, command.index)?;
            vm.general
                .set_storage_policy(value)
                .map_err(|error| error.to_string())
        }
        "video.vsync" => {
            let value = toggle_value(row, command.index)?;
            vm.video.set_vsync(value).map_err(|error| error.to_string())
        }
        "audio.muted" => {
            let value = toggle_value(row, command.index)?;
            vm.audio.set_mute(value).map_err(|error| error.to_string())
        }
        "audio.volume" => {
            let value: u8 = range_value(&command.key, command.number)?;
            vm.audio
                .set_volume(value)
                .map_err(|error| error.to_string())
        }
        "audio.sample_rate" => {
            let value: u32 = choice(row, command.index)?;
            vm.audio
                .set_sample_rate(value)
                .map_err(|error| error.to_string())
        }
        "audio.latency" => {
            let value: u16 = range_value(&command.key, command.number)?;
            vm.audio
                .set_latency(value)
                .map_err(|error| error.to_string())
        }
        "notifications.info_enabled" => {
            let value = toggle_value(row, command.index)?;
            vm.notifications
                .set_info_enabled(value)
                .map_err(|error| error.to_string())
        }
        "notifications.info_duration" => {
            let value: u32 = range_value(&command.key, command.number)?;
            vm.notifications
                .set_info_duration_ms(value)
                .map_err(|error| error.to_string())
        }
        "notifications.info_color" => {
            let value: NotificationColor = choice(row, command.index)?;
            vm.notifications
                .set_info_color(value)
                .map_err(|error| error.to_string())
        }
        "notifications.warning_enabled" => {
            let value = toggle_value(row, command.index)?;
            vm.notifications
                .set_warning_enabled(value)
                .map_err(|error| error.to_string())
        }
        "notifications.warning_duration" => {
            let value: u32 = range_value(&command.key, command.number)?;
            vm.notifications
                .set_warning_duration_ms(value)
                .map_err(|error| error.to_string())
        }
        "notifications.warning_color" => {
            let value: NotificationColor = choice(row, command.index)?;
            vm.notifications
                .set_warning_color(value)
                .map_err(|error| error.to_string())
        }
        key if key.starts_with("pad.") && key.ends_with(".deadzone") => {
            let pad = pad_screen_index(key)?;
            let value: u8 = range_value(&command.key, command.number)?;
            vm.gamepads
                .set_deadzone(pad, value)
                .map_err(|error| error.to_string())
        }
        key if key.starts_with("pad.") && key.ends_with(".triggers") => {
            let pad = pad_screen_index(key)?;
            let value = toggle_value(row, command.index)?;
            vm.gamepads
                .set_trigger_alias(pad, value)
                .map_err(|error| error.to_string())
        }
        "notifications.error_enabled" => {
            let value = toggle_value(row, command.index)?;
            vm.notifications
                .set_error_enabled(value)
                .map_err(|error| error.to_string())
        }
        "notifications.error_modal" => {
            let value = toggle_value(row, command.index)?;
            vm.notifications
                .set_error_modal(value)
                .map_err(|error| error.to_string())
        }
        "notifications.error_color" => {
            let value: NotificationColor = choice(row, command.index)?;
            vm.notifications
                .set_error_color(value)
                .map_err(|error| error.to_string())
        }
        "notifications.position" => {
            let value: NotificationPosition = choice(row, command.index)?;
            vm.notifications
                .set_position(value)
                .map_err(|error| error.to_string())
        }
        "notifications.area_mode" => {
            let value: NotificationAreaMode = choice(row, command.index)?;
            vm.notifications
                .set_area_mode(value)
                .map_err(|error| error.to_string())
        }
        "status.enabled" => {
            let value = toggle_value(row, command.index)?;
            vm.notifications
                .set_status_enabled(value)
                .map_err(|error| error.to_string())
        }
        "status.position" => {
            let value: NotificationPosition = choice(row, command.index)?;
            vm.notifications
                .set_status_position(value)
                .map_err(|error| error.to_string())
        }
        "status.area_mode" => {
            let value: NotificationAreaMode = choice(row, command.index)?;
            vm.notifications
                .set_status_area_mode(value)
                .map_err(|error| error.to_string())
        }
        key => {
            if let Some(rest) = key.strip_prefix("system.") {
                apply_system_command(vm, deps, rest, row, command)
            } else {
                Err(format!("unknown settings key: {key}"))
            }
        }
    }
}

fn apply_system_command(
    vm: &SettingsViewModel,
    deps: &VmDeps,
    rest: &str,
    row: &RowWire,
    command: &WireCommand,
) -> Result<(), String> {
    if let Some((system, slot)) = rest.split_once(".slot.") {
        return apply_slot_command(vm, deps, system, slot, row, command);
    }
    let (system, field) = rest
        .split_once('.')
        .ok_or_else(|| format!("malformed system key: {}", command.key))?;
    let target = vm
        .systems()
        .iter()
        .find(|view| view.system_id().to_string() == system)
        .ok_or_else(|| format!("unknown system: {system}"))?;
    let choice_id: String = choice(row, command.index)?;
    target
        .set_choice(
            &SystemSettingsFieldId(field.to_string().into()),
            &SystemSettingsChoiceId(choice_id.into()),
        )
        .map_err(|error| error.to_string())
}

fn apply_slot_command(
    vm: &SettingsViewModel,
    deps: &VmDeps,
    system: &str,
    slot: &str,
    row: &RowWire,
    command: &WireCommand,
) -> Result<(), String> {
    let factory = deps
        .factories
        .iter()
        .find(|factory| factory.system_id().to_string() == system)
        .ok_or_else(|| format!("unknown system: {system}"))?;
    let attachment = factory
        .input_system_factory()
        .resolve_slot(slot)
        .ok_or_else(|| format!("unknown slot: {slot}"))?;
    let target = vm
        .inputs()
        .iter()
        .find(|view| view.system_id().to_string() == system)
        .ok_or_else(|| format!("unknown system: {system}"))?;
    let profile: Option<String> = choice(row, command.index)?;
    target
        .set_controller_slot(attachment, profile.as_deref())
        .map_err(|error| error.to_string())
}

/// Returns true when the key is Android-local (handled here); false
/// falls through to device/view-model routing, which fails loud on typos.
fn apply_android_local(
    snapshot: &mut SettingsSnapshot,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<bool, String> {
    let row = match find_row(sections, &command.key) {
        Ok(row) => row,
        Err(_) => return Ok(false),
    };
    let local = &mut snapshot.local;
    match command.key.as_str() {
        "video.orientation" => {
            local.screen_orientation = choice(row, command.index)?;
        }
        "controls.overlay_visibility" => {
            local.touch_overlay.visibility = choice(row, command.index)?;
        }
        "controls.overlay_opacity" => {
            local.touch_overlay.opacity_percent = range_value(&command.key, command.number)?;
        }
        "controls.overlay_scale" => {
            local.touch_overlay.scale_percent = range_value(&command.key, command.number)?;
        }
        "controls.overlay_vertical_offset" => {
            local.touch_overlay.vertical_offset_percent =
                range_value(&command.key, command.number)?;
        }
        "controls.overlay_haptics" => {
            local.touch_overlay.haptics = toggle_value(row, command.index)?;
        }
        "controls.cartridge_motion" => {
            local.cartridge_peripherals.motion_enabled = toggle_value(row, command.index)?;
        }
        "controls.cartridge_rumble" => {
            local.cartridge_peripherals.rumble_enabled = toggle_value(row, command.index)?;
        }
        "controls.cartridge_rumble_strength" => {
            local.cartridge_peripherals.rumble_strength_percent =
                range_value(&command.key, command.number)?;
        }
        "controls.cartridge_rumble_target" => {
            local.cartridge_peripherals.rumble_target = choice(row, command.index)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn req_str<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(|node| node.as_str())
        .ok_or_else(|| format!("capture target misses {field}"))
}

fn req_system(
    deps: &VmDeps,
    value: &serde_json::Value,
) -> Result<Box<dyn nerust_core_traits::identity::SystemId>, String> {
    let system = req_str(value, "system")?;
    deps.factories
        .iter()
        .find(|factory| factory.system_id().to_string() == system)
        .map(|factory| factory.system_id())
        .ok_or_else(|| format!("unknown system: {system}"))
}

fn req_pad(value: &serde_json::Value) -> Result<usize, String> {
    let pad = value
        .get("pad")
        .and_then(|pad| pad.as_u64())
        .ok_or_else(|| "capture target misses pad".to_string())?;
    let pad: usize = pad
        .try_into()
        .map_err(|_| "capture target pad out of range".to_string())?;
    if pad >= nerust_input_traits::MAX_PADS {
        return Err(format!("capture target pad out of range: {pad}"));
    }
    Ok(pad)
}

fn decode_target_value(value: &serde_json::Value, deps: &VmDeps) -> Result<CaptureTarget, String> {
    match value.get("kind").and_then(|kind| kind.as_str()) {
        Some("binding") => {
            let system = req_system(deps, value)?;
            Ok(CaptureTarget::Binding {
                system,
                attachment: req_str(value, "attachment")?.to_string(),
                control: req_str(value, "control")?.to_string(),
            })
        }
        Some("shortcut") => {
            let raw = value
                .get("action")
                .cloned()
                .ok_or_else(|| "capture target misses action".to_string())?;
            let action: nerust_settings_core::model::input::ShortcutAction =
                serde_json::from_value(raw).map_err(|error| error.to_string())?;
            Ok(CaptureTarget::Shortcut(action))
        }
        Some("pad_binding") => {
            let system = req_system(deps, value)?;
            Ok(CaptureTarget::PadBinding {
                system,
                pad: req_pad(value)?,
                attachment: req_str(value, "attachment")?.to_string(),
                control: req_str(value, "control")?.to_string(),
            })
        }
        _ => Err("unknown capture target".to_string()),
    }
}

/// Opaque capture target: the row string parses exactly once per command;
/// inner boundaries pass the `Value` through without re-serializing.
fn decode_target(encoded: &str, deps: &VmDeps) -> Result<CaptureTarget, String> {
    let value: serde_json::Value =
        serde_json::from_str(encoded).map_err(|error| error.to_string())?;
    decode_target_value(&value, deps)
}
