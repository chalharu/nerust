//! Android settings views: ViewModel-DTO-driven wire model and stateless orchestration.
//!
//! The Android settings screen is a dumb renderer over [`SettingsViewModel`]
//! projections. Every entry builds a fresh view model from the snapshot the
//! caller passes in, applies at most one command, and drops it before
//! returning: no resident editor crosses the JNI boundary, so the `!Send`
//! editor never leaves its calling thread.
//!
//! Onion: transport lives here, policy lives in the view model. Kotlin only
//! renders sections/rows and echoes `(key, index, number)` commands back.

use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::{SettingsViewModel, StoragePathError, StoragePathValidator};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{
        UiText, port_count_line, port_summary, shortcut_row_description, slot_description,
        system_field_description, text as ui_text,
    },
    input::{BUILTIN_KEYBOARD_MEMBER, GAMEPAD_MEMBER_PREFIX, KEYBOARD_MEMBER_PREFIX},
    model::{
        app_state::{DesktopAppState, PORT_PLAYER_1, PORT_PLAYER_2},
        input::{default_shortcut_bindings, ensure_default_shortcuts},
        language::AppLanguage,
        local::{
            HostBackendLocalSettings, NotificationAreaMode, NotificationColor,
            NotificationPosition, RumbleTarget, ScreenOrientation, TouchOverlayVisibility,
        },
        shared::{DesktopSharedSettings, StoragePolicy},
    },
};
use nerust_system_traits::factory::{
    CoreFactory,
    descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
};
use serde::{Deserialize, Serialize};

/// Wire contract version for the DTO-driven settings screen.
/// 1 was the descriptor/codec dialog; 2 is the ViewModel projection.
pub(super) const SETTINGS_SCHEMA_VERSION: u32 = 2;

/// Full screen payload: schema version, sections Kotlin renders, live
/// validation issues for the draft, and the pending key-capture state.
/// The draft snapshot stays in the bridge vault; Kotlin never sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ViewPayload {
    pub schema_version: u32,
    pub sections: Vec<SectionWire>,
    pub issues: Vec<String>,
    pub capture: Option<CaptureWire>,
}

/// Pending key capture: opaque target plus the localized prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CaptureWire {
    pub target: String,
    pub prompt: String,
}

/// One settings section: id plus pre-resolved label plus rows. Invisible
/// sections stay addressable (group navigation) but leave the category list.
/// `sub` is the navigation breadcrumb parent ("Player 1" under "Devices");
/// empty falls back to the top-level title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SectionWire {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub sub: String,
    pub rows: Vec<RowWire>,
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// One settings row. `choices` carries both the Kotlin-visible labels and the
/// Rust-typed values (echoed back by index); `number`/`step`/`min`/`max`/`unit`
/// drive slider rows; `text` drives read-only info rows and doubles as the
/// trailing value for `group` rows (player/device summaries) and as nothing
/// for `button` rows; `description` is the M3 supporting text shown under the
/// label (IA principle 3: every non-header row explains itself, except info
/// readouts whose label is self-evident and capture rows whose value line
/// already serves as the副文); `detail_target` shows a trailing Details
/// button navigating to another section (device rows); `target` carries the
/// opaque capture target for `capture` rows; `Header`/`Group` rows are
/// non-editable navigation aids. `Button` rows ask Kotlin for confirmation
/// first, then send the `reset_section` op named by the row key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RowWire {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub kind: RowKind,
    #[serde(default)]
    pub detail_target: String,
    #[serde(default)]
    pub choices: Vec<ChoiceWire>,
    #[serde(default)]
    pub selected: usize,
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub step: i64,
    #[serde(default)]
    pub min: i64,
    #[serde(default)]
    pub max: i64,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub target: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

/// Row presentation kinds. Toggles render as switches, numbers as sliders,
/// infos as static text, headers as subgroup titles, groups as navigation
/// rows into another section, captures as key-capture rows, buttons as
/// confirm-then-run actions (resets); choices render as radio lists.
/// A non-empty `detail_target` adds a trailing Details button (device rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RowKind {
    Choice,
    Toggle,
    Number,
    Info,
    Header,
    Group,
    Capture,
    Button,
}

/// One pick-list entry: display label, opaque typed value, whether the
/// choice is currently selectable (slot conflicts disable taken profiles),
/// and the owner slot label when disabled so Kotlin can explain why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChoiceWire {
    pub value: serde_json::Value,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub reason: String,
}

/// One physical keyboard enumerated on the device (Kotlin `InputDevice`).
/// The vault holds the list; Rust never touches hardware. `member` is the
/// stable port-set id ("kbd:builtin" or "kbd:<descriptor>"), `name` the
/// OS display name, `builtin` the built-in row (device id 0, always
/// present when it exists; external rows are per descriptor, BT/USB
/// agnostic, virtual devices excluded, HID duplicate descriptors merged —
/// all classified Kotlin-side, see the IA keyboard rule).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct KeyboardInfo {
    pub member: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
}

/// One settings edit from Kotlin: row key plus op. `set` reads the choice
/// index or the slider number; capture ops act on the row's opaque target;
/// `keyboards` upserts the vault's enumerated keyboard list from `payload`;
/// `reset_section` restores one screen (or `all`) to defaults.
/// `payload` is op-specific JSON (`keyboards` only today).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireCommand {
    pub key: String,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub number: i64,
    #[serde(default = "default_op_set")]
    pub op: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

fn default_op_set() -> String {
    "set".to_string()
}

/// View-model dependencies shared across JNI calls. `CoreFactory` is
/// `Send + Sync`; the snapshot travels per call, so no editor state is kept.
pub(super) struct VmDeps {
    pub factories: Vec<Arc<dyn CoreFactory>>,
    pub rates: Arc<[u32]>,
}

static VM_DEPS: Mutex<Option<VmDeps>> = Mutex::new(None);

/// Publish the factories and probed sample rates once at startup.
/// Called from the event thread; JNI commands only read through the lock.
pub(super) fn install_vm_deps(factories: Vec<Arc<dyn CoreFactory>>, rates: Arc<[u32]>) {
    match VM_DEPS.lock() {
        Ok(mut slot) => *slot = Some(VmDeps { factories, rates }),
        Err(poison) => {
            log::error!("android settings vm deps lock poisoned; recovering state");
            *poison.into_inner() = Some(VmDeps { factories, rates });
        }
    }
}

fn with_vm_deps<T>(operation: impl FnOnce(&VmDeps) -> Result<T, String>) -> Result<T, String> {
    let guard = VM_DEPS
        .lock()
        .map_err(|_| "android settings vm deps lock poisoned".to_string())?;
    let Some(deps) = guard.as_ref() else {
        return Err("android settings not initialized".to_string());
    };
    operation(deps)
}

/// Android storage lives behind SAF: the document-tree picker flow owns
/// existence, so this only rejects malformed paths. Snapshot-level URI rules
/// still run in the view-model validator.
#[derive(Debug)]
struct SafStorageValidator;

impl StoragePathValidator for SafStorageValidator {
    fn validate(&self, path: &std::path::Path) -> Result<(), StoragePathError> {
        let text = path.to_string_lossy();
        if text.is_empty() || text.contains('\0') {
            return Err(StoragePathError::Inaccessible(
                "storage path must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

fn build_vm(snapshot: &SettingsSnapshot, deps: &VmDeps) -> Result<SettingsViewModel, String> {
    SettingsViewModel::new(
        snapshot.clone(),
        deps.factories.clone(),
        Arc::clone(&deps.rates),
        Rc::new(SafStorageValidator),
    )
    .map_err(|error| error.to_string())
}

/// Open payload for Kotlin: schema version, sections, live issues from a
/// non-fatal `finish` probe, and the pending key-capture state.
/// `keyboards` is the vault's enumerated physical-keyboard list;
/// `touch_capable` is `None` until Kotlin reports touch capability.
pub(super) fn open_payload(
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Result<String, String> {
    with_vm_deps(|deps| {
        let vm = build_vm(snapshot, deps)?;
        let payload = ViewPayload {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sections: build_sections(&vm, snapshot, keyboards, touch_capable),
            issues: draft_issues(&vm),
            capture: None,
        };
        serde_json::to_string(&payload).map_err(|error| error.to_string())
    })
}

/// Pending capture slot: target plus the localized prompt captured at start.
pub(super) type CaptureSlot = Option<(CaptureTarget, String)>;

/// Apply one wire command to the draft snapshot in place, returning the
/// refreshed payload. Stateless per call except for the passed draft,
/// capture slot, vault keyboard list (`keyboards` op upserts it), and
/// touch capability (`touch` op upserts it).
pub(super) fn apply_command_to(
    snapshot: &mut SettingsSnapshot,
    command_json: &str,
    capture: &mut CaptureSlot,
    keyboards: &mut Vec<KeyboardInfo>,
    touch_capable: &mut Option<bool>,
) -> Result<String, String> {
    let command: WireCommand =
        serde_json::from_str(command_json).map_err(|error| error.to_string())?;
    with_vm_deps(|deps| {
        match command.op.as_str() {
            "keyboards" => {
                apply_keyboards_command(keyboards, &command)?;
                prune_missing_builtin(snapshot, keyboards);
            }
            "touch" => apply_touch_command(touch_capable, &command)?,
            "reset_section" => {
                let vm = build_vm(snapshot, deps)?;
                let sections = build_sections(&vm, snapshot, keyboards, *touch_capable);
                apply_reset_section(
                    snapshot,
                    deps,
                    &sections,
                    &command,
                    keyboards,
                    *touch_capable,
                )?;
            }
            "cancel_capture" | "clear_binding" | "start_capture" => {
                apply_capture_command(snapshot, deps, &command, capture, keyboards, *touch_capable)?
            }
            "set" => {
                let vm = build_vm(snapshot, deps)?;
                let sections = build_sections(&vm, snapshot, keyboards, *touch_capable);
                apply_set_command(snapshot, deps, &vm, &sections, &command)?;
            }
            op => return Err(format!("unknown command op: {op}")),
        }
        refreshed_payload(snapshot, deps, capture, keyboards, *touch_capable)
    })
}

/// One `set` edit: route by key family (Android-local overlay/cartridge
/// keys, port device membership, everything else through the view model).
/// Shared by live commands and reset replays so both take the same path.
fn apply_set_command(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    vm: &SettingsViewModel,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    if is_android_local_key(&command.key) {
        apply_android_local(snapshot, sections, command)
    } else if is_device_key(&command.key) {
        apply_device_command(snapshot, sections, command)
    } else {
        apply_view_model_command(vm, deps, sections, command)?;
        *snapshot = vm.snapshot();
        Ok(())
    }
}

/// Refreshed payload after any draft mutation: sections, live issues from a
/// non-fatal `finish` probe, and the pending capture state.
fn refreshed_payload(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    capture: &CaptureSlot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Result<String, String> {
    let vm = build_vm(snapshot, deps)?;
    let payload = ViewPayload {
        schema_version: SETTINGS_SCHEMA_VERSION,
        sections: build_sections(&vm, snapshot, keyboards, touch_capable),
        issues: draft_issues(&vm),
        capture: capture.as_ref().map(|(target, prompt)| CaptureWire {
            target: encode_target(target, &target_system(target, deps)),
            prompt: prompt.clone(),
        }),
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// Hardware key routing for capture (and future keyboard play): Android
/// keycodes to `Key`. Only pressed events with an active capture mutate.
/// Returns the refreshed payload plus whether the key was consumed as a
/// binding. Releases and unmapped keycodes leave the capture pending and
/// report `false` so the platform can handle them (volume, Back, ...).
pub(super) fn apply_hardware_key(
    keycode: i32,
    pressed: bool,
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Result<(String, bool), String> {
    with_vm_deps(|deps| {
        if pressed
            && let Some((target, _)) = capture.clone()
            && let Some(key) = android_keycode_to_key(keycode)
        {
            let vm = build_vm(snapshot, deps)?;
            vm.capture
                .start_capture(target)
                .map_err(|error| error.to_string())?;
            vm.capture.apply_captured_key(key);
            *snapshot = vm.snapshot();
            *capture = None;
            Ok((
                refreshed_payload(snapshot, deps, capture, keyboards, touch_capable)?,
                true,
            ))
        } else {
            Ok((
                refreshed_payload(snapshot, deps, capture, keyboards, touch_capable)?,
                false,
            ))
        }
    })
}

fn android_keycode_to_key(keycode: i32) -> Option<nerust_input_traits::Key> {
    use nerust_input_traits::Key;
    Some(match keycode {
        7 => Key::Digit0,
        8 => Key::Digit1,
        9 => Key::Digit2,
        10 => Key::Digit3,
        11 => Key::Digit4,
        12 => Key::Digit5,
        13 => Key::Digit6,
        14 => Key::Digit7,
        15 => Key::Digit8,
        16 => Key::Digit9,
        19 => Key::ArrowUp,
        20 => Key::ArrowDown,
        21 => Key::ArrowLeft,
        22 => Key::ArrowRight,
        29 => Key::KeyA,
        30 => Key::KeyB,
        31 => Key::KeyC,
        32 => Key::KeyD,
        33 => Key::KeyE,
        34 => Key::KeyF,
        35 => Key::KeyG,
        36 => Key::KeyH,
        37 => Key::KeyI,
        38 => Key::KeyJ,
        39 => Key::KeyK,
        40 => Key::KeyL,
        41 => Key::KeyM,
        42 => Key::KeyN,
        43 => Key::KeyO,
        44 => Key::KeyP,
        45 => Key::KeyQ,
        46 => Key::KeyR,
        47 => Key::KeyS,
        48 => Key::KeyT,
        49 => Key::KeyU,
        50 => Key::KeyV,
        51 => Key::KeyW,
        52 => Key::KeyX,
        53 => Key::KeyY,
        54 => Key::KeyZ,
        55 => Key::Comma,
        56 => Key::Period,
        57 => Key::AltLeft,
        58 => Key::AltRight,
        59 => Key::ShiftLeft,
        60 => Key::ShiftRight,
        61 => Key::Tab,
        62 => Key::Space,
        66 => Key::Enter,
        67 => Key::Backspace,
        68 => Key::Backquote,
        69 => Key::Minus,
        70 => Key::Equal,
        71 => Key::BracketLeft,
        72 => Key::BracketRight,
        73 => Key::Backslash,
        74 => Key::Semicolon,
        75 => Key::Quote,
        76 => Key::Slash,
        92 => Key::PageUp,
        93 => Key::PageDown,
        111 => Key::Escape,
        112 => Key::Delete,
        113 => Key::ControlLeft,
        114 => Key::ControlRight,
        115 => Key::CapsLock,
        122 => Key::Home,
        123 => Key::End,
        124 => Key::Insert,
        131 => Key::F1,
        132 => Key::F2,
        133 => Key::F3,
        134 => Key::F4,
        135 => Key::F5,
        136 => Key::F6,
        137 => Key::F7,
        138 => Key::F8,
        139 => Key::F9,
        140 => Key::F10,
        141 => Key::F11,
        142 => Key::F12,
        144 => Key::Numpad0,
        145 => Key::Numpad1,
        146 => Key::Numpad2,
        147 => Key::Numpad3,
        148 => Key::Numpad4,
        149 => Key::Numpad5,
        150 => Key::Numpad6,
        151 => Key::Numpad7,
        152 => Key::Numpad8,
        153 => Key::Numpad9,
        154 => Key::NumpadDivide,
        155 => Key::NumpadMultiply,
        156 => Key::NumpadSubtract,
        157 => Key::NumpadAdd,
        158 => Key::NumpadDecimal,
        163 => Key::NumpadEnter,
        164 => Key::NumpadEqual,
        _ => return None,
    })
}

fn target_system(target: &CaptureTarget, deps: &VmDeps) -> String {
    match target {
        CaptureTarget::Binding { system, .. } => system.to_string(),
        CaptureTarget::Shortcut(_) => deps
            .factories
            .first()
            .map(|factory| factory.system_id().to_string())
            .unwrap_or_default(),
    }
}

/// Non-fatal validation probe: the same gate `finish` enforces at save,
/// surfaced live so the UI can explain disabled/unappliable states.
fn draft_issues(vm: &SettingsViewModel) -> Vec<String> {
    match vm.finish() {
        Ok(_) => Vec::new(),
        Err(state) => state
            .issues
            .iter()
            .map(|issue| issue.message.clone())
            .collect(),
    }
}

/// Capture ops: start/clear/cancel against the vault slot. Binding targets
/// decode from the row's opaque target; the row key only locates the row.
/// Clears mutate the draft through the VM snapshot round-trip, exactly like
/// every other setter.
fn apply_capture_command(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    command: &WireCommand,
    capture: &mut CaptureSlot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Result<(), String> {
    if command.op == "cancel_capture" {
        *capture = None;
        return Ok(());
    }
    let vm = build_vm(snapshot, deps)?;
    let sections = build_sections(&vm, snapshot, keyboards, touch_capable);
    let row = find_row(&sections, &command.key)?;
    if row.kind != RowKind::Capture {
        return Err(format!("{}: not a capture row", command.key));
    }
    let target = decode_target(&row.target, deps)?;
    match command.op.as_str() {
        "clear_binding" => {
            vm.capture
                .clear_binding(&target)
                .map_err(|error| error.to_string())?;
            *snapshot = vm.snapshot();
            *capture = None;
            Ok(())
        }
        "start_capture" => {
            vm.capture
                .start_capture(target.clone())
                .map_err(|error| error.to_string())?;
            let prompt = vm.capture.view.get().prompt.clone();
            *capture = Some((target, prompt));
            Ok(())
        }
        op => Err(format!("unknown command op: {op}")),
    }
}

/// Record whether the terminal has a touch screen (Kotlin
/// `PackageManager`, re-pushed with the keyboard enumeration). `None`
/// means unknown: rows behave as before, the runtime keeps showing.
fn apply_touch_command(
    touch_capable: &mut Option<bool>,
    command: &WireCommand,
) -> Result<(), String> {
    let capable: bool = serde_json::from_value(
        command
            .payload
            .get("capable")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|error| error.to_string())?;
    *touch_capable = Some(capable);
    Ok(())
}

/// Upsert the vault's enumerated physical-keyboard list (Kotlin
/// `InputDevice` enumeration, hotplug refresh included). Not a draft
/// mutation: the list is ephemeral device truth, never persisted.
fn apply_keyboards_command(
    keyboards: &mut Vec<KeyboardInfo>,
    command: &WireCommand,
) -> Result<(), String> {
    let list: Vec<KeyboardInfo> =
        serde_json::from_value(command.payload.clone()).map_err(|error| error.to_string())?;
    for info in &list {
        if info.member.is_empty() {
            return Err("keyboard member id must not be empty".to_string());
        }
    }
    *keyboards = list;
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
    if !PLAYER_PORTS.iter().any(|(known, ..)| *known == port) {
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
/// defaults by diffing the live rows against a default projection and
/// replaying the differing sets through the normal setters. Capture rows
/// reset to unbound (the default profile binds nothing); shortcut rows
/// restore their canonical default keys.
fn apply_reset_section(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    sections: &[SectionWire],
    command: &WireCommand,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Result<(), String> {
    let scope = command
        .key
        .strip_prefix("reset.")
        .ok_or_else(|| format!("malformed reset key: {}", command.key))?;
    let default = default_snapshot(deps);
    let default_vm = build_vm(&default, deps)?;
    let default_sections = build_sections(&default_vm, &default, keyboards, touch_capable);
    let reset_all = scope == "all";
    if reset_all {
        // Membership rows only cover enumerated keyboards; stale members
        // have no rows, so restore the whole port map wholesale.
        snapshot.app_state.port_devices = default.app_state.port_devices.clone();
    }
    if !reset_all && sections.iter().all(|section| section.id != scope) {
        return Err(format!("unknown reset scope: {scope}"));
    }
    for section in sections {
        if !reset_all && section.id != scope {
            continue;
        }
        for row in &section.rows {
            let default_row = match default_sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|default_row| default_row.key == row.key)
            {
                Some(default_row) => default_row,
                // Rows without a default counterpart (device membership,
                // enumerated keyboards) are left alone.
                None => continue,
            };
            reset_row(snapshot, deps, sections, row, default_row)?;
        }
    }
    Ok(())
}

fn reset_row(
    snapshot: &mut SettingsSnapshot,
    deps: &VmDeps,
    sections: &[SectionWire],
    row: &RowWire,
    default_row: &RowWire,
) -> Result<(), String> {
    debug_assert_eq!(row.key, default_row.key);
    match row.kind {
        RowKind::Choice | RowKind::Toggle | RowKind::Number => {
            let differs = if matches!(row.kind, RowKind::Number) {
                row.number != default_row.number
            } else {
                row.selected != default_row.selected
            };
            if differs {
                let reset = WireCommand {
                    key: row.key.clone(),
                    index: default_row.selected,
                    number: default_row.number,
                    op: "set".to_string(),
                    payload: serde_json::Value::Null,
                };
                let vm = build_vm(snapshot, deps)?;
                apply_set_command(snapshot, deps, &vm, sections, &reset)?;
            }
            Ok(())
        }
        RowKind::Capture => {
            if row.text != default_row.text {
                let vm = build_vm(snapshot, deps)?;
                let target = decode_target(&row.target, deps)?;
                match &target {
                    CaptureTarget::Shortcut(action) => {
                        vm.capture
                            .clear_binding(&target)
                            .map_err(|error| error.to_string())?;
                        if let Some(key) = default_shortcut_bindings()
                            .iter()
                            .find(|binding| binding.action == *action)
                            .and_then(|binding| binding.key)
                        {
                            vm.capture
                                .start_capture(target)
                                .map_err(|error| error.to_string())?;
                            vm.capture.apply_captured_key(key);
                        }
                        *snapshot = vm.snapshot();
                    }
                    CaptureTarget::Binding { .. } => {
                        // Default profiles bind nothing: clear suffices.
                        vm.capture
                            .clear_binding(&target)
                            .map_err(|error| error.to_string())?;
                        *snapshot = vm.snapshot();
                    }
                }
            }
            Ok(())
        }
        RowKind::Header | RowKind::Info | RowKind::Group | RowKind::Button => Ok(()),
    }
}

/// Validate the draft through the view model. Conflicts and factory rules run here.
pub(super) fn finish_snapshot(
    snapshot: &SettingsSnapshot,
    factories: &[Arc<dyn CoreFactory>],
    rates: Arc<[u32]>,
) -> Result<SettingsSnapshot, Vec<String>> {
    let vm = SettingsViewModel::new(
        snapshot.clone(),
        factories.to_vec(),
        rates,
        Rc::new(SafStorageValidator),
    )
    .map_err(|error| vec![error.to_string()])?;
    vm.finish().map_err(|state| {
        state
            .issues
            .iter()
            .map(|issue| issue.message.clone())
            .collect()
    })
}

fn is_android_local_key(key: &str) -> bool {
    matches!(
        key,
        "video.orientation"
            | "controls.overlay_visibility"
            | "controls.overlay_opacity"
            | "controls.overlay_scale"
            | "controls.overlay_vertical_offset"
            | "controls.overlay_haptics"
            | "controls.cartridge_motion"
            | "controls.cartridge_rumble"
            | "controls.cartridge_rumble_strength"
            | "controls.cartridge_rumble_target"
    )
}

fn choice<T: serde::de::DeserializeOwned>(row: &RowWire, index: usize) -> Result<T, String> {
    row.choices
        .get(index)
        .ok_or_else(|| format!("{}: choice index {index} out of range", row.key))
        .and_then(|choice| {
            serde_json::from_value(choice.value.clone())
                .map_err(|error| format!("{}: invalid choice value: {error}", row.key))
        })
}

fn toggle_value(row: &RowWire, index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!("{}: toggle index {index} out of range", row.key)),
    }
}

fn range_value<T>(key: &str, number: i64) -> Result<T, String>
where
    T: TryFrom<i64>,
{
    T::try_from(number).map_err(|_| format!("{key}: value {number} out of range"))
}

fn find_row<'a>(sections: &'a [SectionWire], key: &str) -> Result<&'a RowWire, String> {
    sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.key == key)
        .ok_or_else(|| format!("unknown settings key: {key}"))
}

fn apply_view_model_command(
    vm: &SettingsViewModel,
    deps: &VmDeps,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
    match command.key.as_str() {
        "general.language" => {
            let value: AppLanguage = choice(row, command.index)?;
            vm.general
                .set_language(value)
                .map_err(|error| error.to_string())
        }
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

fn apply_android_local(
    snapshot: &mut SettingsSnapshot,
    sections: &[SectionWire],
    command: &WireCommand,
) -> Result<(), String> {
    let row = find_row(sections, &command.key)?;
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
        key => return Err(format!("unknown settings key: {key}")),
    }
    Ok(())
}

// ── Section builders ────────────────────────────────────────────────

fn choice_row<T>(
    key: &str,
    label: String,
    description: String,
    choices: &[(T, String)],
    selected: &T,
) -> RowWire
where
    T: Serialize + PartialEq,
{
    let selected_value = serde_json::to_value(selected).unwrap_or(serde_json::Value::Null);
    let wires: Vec<ChoiceWire> = choices
        .iter()
        .map(|(value, label)| ChoiceWire {
            value: serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            label: label.clone(),
            enabled: true,
            reason: String::new(),
        })
        .collect();
    let selected = wires
        .iter()
        .position(|wire| wire.value == selected_value)
        .unwrap_or_default();
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Choice,
        choices: wires,
        selected,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text: String::new(),
        target: String::new(),
        detail_target: String::new(),
        visible: true,
    }
}

fn toggle_row(
    key: &str,
    label: String,
    description: String,
    language: AppLanguage,
    value: bool,
) -> RowWire {
    let mut row = choice_row(
        key,
        label,
        description,
        &[
            (false, ui_text(language, UiText::Off).to_string()),
            (true, ui_text(language, UiText::On).to_string()),
        ],
        &value,
    );
    row.selected = usize::from(value);
    row.kind = RowKind::Toggle;
    row
}

#[allow(
    clippy::too_many_arguments,
    reason = "Wire builder: slider bounds read as one tuple"
)]
fn number_row(
    key: &str,
    label: String,
    description: String,
    value: i64,
    step: i64,
    min: i64,
    max: i64,
    unit: &str,
) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Number,
        choices: Vec::new(),
        selected: 0,
        number: value,
        step,
        min,
        max,
        unit: unit.to_string(),
        text: String::new(),
        target: String::new(),
        detail_target: String::new(),
        visible: true,
    }
}

fn info_row(key: &str, label: String, description: String, text: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Info,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text,
        target: String::new(),
        detail_target: String::new(),
        visible: true,
    }
}

fn header_row(label: String) -> RowWire {
    RowWire {
        key: String::new(),
        label,
        description: String::new(),
        kind: RowKind::Header,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text: String::new(),
        target: String::new(),
        detail_target: String::new(),
        visible: true,
    }
}

/// Navigation row into another section by id.
fn group_row(key: &str, label: String, description: String, target: &str) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Group,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text: String::new(),
        target: target.to_string(),
        detail_target: String::new(),
        visible: true,
    }
}

/// Key-capture row: tap starts capture against the opaque target, Clear
/// unbinds, Cancel aborts. `text` shows the current binding label.
/// `description` stays empty: the value line already serves as the副文
/// (bound key name, or "Tap to assign").
fn capture_row(
    key: &str,
    label: String,
    description: String,
    text: String,
    target: &str,
) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Capture,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text,
        target: target.to_string(),
        detail_target: String::new(),
        visible: true,
    }
}

/// Confirm-then-run action row (resets). The row key names the scope:
/// `reset.all` or `reset.<section-id>`. Kotlin confirms first, then sends
/// the `reset_section` op; Rust restores that scope from defaults.
fn button_row(key: &str, label: String, description: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Button,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        min: 0,
        max: 0,
        unit: String::new(),
        text: String::new(),
        target: String::new(),
        detail_target: String::new(),
        visible: true,
    }
}

/// Global player ports: port id, player screen, device picker, type screen.
/// Ports are global (not per machine): every system's player-N slot fans
/// out from the same port set.
const PLAYER_PORTS: [(&str, &str, &str, &str); 2] = [
    (PORT_PLAYER_1, "player.1", "devices.1", "types.1"),
    (PORT_PLAYER_2, "player.2", "devices.2", "types.2"),
];

fn player_label(index: usize, language: AppLanguage) -> String {
    ui_text(
        language,
        if index == 0 {
            UiText::PlayerOne
        } else {
            UiText::PlayerTwo
        },
    )
    .to_string()
}

fn build_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Vec<SectionWire> {
    let language = snapshot.shared.general.language;
    let mut sections = vec![
        build_play_section(vm, snapshot, language),
        build_saves_section(vm, language),
        build_notifications_section(vm, language),
        build_app_section(vm, language),
        build_advanced_section(vm, snapshot, language),
        build_about_section(language),
        build_audio_detail_section(vm, language),
    ];
    sections.extend(build_controllers_sections(
        vm,
        snapshot,
        language,
        keyboards,
        touch_capable,
    ));
    sections.extend(build_player_sections(vm, snapshot, language, keyboards));
    sections.extend(build_system_sections(vm, language));
    sections.extend(build_input_sections(vm, language));
    sections.extend(build_keys_sections(vm, language));
    // Canonical order owned by Rust; dynamic device sections (device.*)
    // keep build order after the known taxonomy.
    sections.sort_by_key(|section| {
        crate::settings_sections::SETTINGS_SECTION_IDS
            .iter()
            .position(|id| *id == section.id)
            .unwrap_or(usize::MAX)
    });
    sections
}

/// Play: what you touch every session. Machine-specific filters stay
/// per machine under Advanced (a filter value cannot be global).
fn build_play_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
) -> SectionWire {
    let audio = vm.audio.view.get();
    let orientation = snapshot.local.screen_orientation;
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "play".to_string(),
        label: crate::settings_sections::section_label("play", language).to_string(),
        sub: String::new(),
        rows: vec![
            choice_row(
                "video.orientation",
                t(UiText::ScreenOrientation),
                t(UiText::DescOrientation),
                &[
                    (
                        ScreenOrientation::Auto,
                        ui_text(language, UiText::AutoRotate).to_string(),
                    ),
                    (
                        ScreenOrientation::Portrait,
                        ui_text(language, UiText::Portrait).to_string(),
                    ),
                    (
                        ScreenOrientation::Landscape,
                        ui_text(language, UiText::Landscape).to_string(),
                    ),
                ],
                &orientation,
            ),
            toggle_row(
                "audio.muted",
                t(UiText::Mute),
                t(UiText::DescMute),
                language,
                audio.muted,
            ),
            number_row(
                "audio.volume",
                t(UiText::MasterVolume),
                t(UiText::DescVolume),
                i64::from(audio.volume_percent),
                1,
                0,
                100,
                "%",
            ),
            button_row(
                "reset.play",
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ),
        ],
        visible: true,
    }
}

/// Saves: where save data lives (storage policy section rehoused).
fn build_saves_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let general = vm.general.view.get();
    let t = |text: UiText| ui_text(language, text).to_string();
    let mut rows = vec![{
        let (choices, selected) = (
            general
                .storage_policy_choices
                .iter()
                .map(|choice| (choice.value, choice.label.clone()))
                .collect::<Vec<_>>(),
            general.storage_policy,
        );
        choice_row(
            "general.storage_policy",
            t(UiText::SaveStoragePolicy),
            t(UiText::DescStoragePolicy),
            &choices,
            &selected,
        )
    }];
    if general.show_storage_directory {
        rows.push(info_row(
            "general.storage_directory",
            t(UiText::SaveStorageDirectory),
            t(UiText::DescStorageDirectory),
            general.storage_directory.clone(),
        ));
    }
    rows.push(button_row(
        "reset.saves",
        t(UiText::ResetScreen),
        t(UiText::DescResetScreen),
    ));
    SectionWire {
        id: "saves".to_string(),
        label: crate::settings_sections::section_label("saves", language).to_string(),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// App: language, about, global reset. The reset button confirms in
/// Kotlin first (`reset.all`), then restores every setting to default.
fn build_app_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.general.view.get();
    let t = |text: UiText| ui_text(language, text).to_string();
    let rows = vec![
        {
            let (choices, selected) = (
                view.language_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                view.language,
            );
            choice_row(
                "general.language",
                t(UiText::Language),
                t(UiText::DescLanguage),
                &choices,
                &selected,
            )
        },
        group_row(
            "app.about",
            t(UiText::About),
            t(UiText::DescLicense),
            "about",
        ),
        button_row(
            "reset.all",
            t(UiText::ResetSettings),
            t(UiText::DescResetSettings),
        ),
    ];
    SectionWire {
        id: "app".to_string(),
        label: crate::settings_sections::section_label("app", language).to_string(),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// About: version and license readouts (info rows need no description:
/// the label is self-evident).
fn build_about_section(language: AppLanguage) -> SectionWire {
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "about".to_string(),
        label: t(UiText::About),
        sub: t(UiText::SectionApp),
        rows: vec![
            info_row(
                "about.version",
                t(UiText::Version),
                String::new(),
                format!("nerust {}", env!("CARGO_PKG_VERSION")),
            ),
            info_row(
                "about.license",
                t(UiText::License),
                String::new(),
                "MPL-2.0".to_string(),
            ),
        ],
        visible: false,
    }
}

/// Advanced: rarely touched settings. VSync inline; sample rate/latency
/// behind the audio-detail screen; machine sections behind machine rows.
fn build_advanced_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
) -> SectionWire {
    let video = vm.video.view.get();
    let peripherals = &snapshot.local.cartridge_peripherals;
    let t = |text: UiText| ui_text(language, text).to_string();
    let mut rows = vec![
        toggle_row(
            "video.vsync",
            t(UiText::Vsync),
            t(UiText::DescVsync),
            language,
            video.vsync,
        ),
        group_row(
            "advanced.audio_detail",
            t(UiText::AudioDetail),
            t(UiText::DescLatency),
            "audio.detail",
        ),
        toggle_row(
            "controls.cartridge_motion",
            t(UiText::CartridgeMotion),
            t(UiText::DescCartridgeMotion),
            language,
            peripherals.motion_enabled,
        ),
        toggle_row(
            "controls.cartridge_rumble",
            t(UiText::CartridgeRumble),
            t(UiText::DescCartridgeRumble),
            language,
            peripherals.rumble_enabled,
        ),
        number_row(
            "controls.cartridge_rumble_strength",
            t(UiText::RumbleStrength),
            t(UiText::DescRumbleStrength),
            i64::from(peripherals.rumble_strength_percent),
            1,
            0,
            100,
            "%",
        ),
        choice_row(
            "controls.cartridge_rumble_target",
            t(UiText::RumbleTarget),
            t(UiText::DescRumbleTarget),
            &[
                (
                    RumbleTarget::Auto,
                    ui_text(language, UiText::Auto).to_string(),
                ),
                (
                    RumbleTarget::Handset,
                    ui_text(language, UiText::Handset).to_string(),
                ),
                (
                    RumbleTarget::Controller,
                    ui_text(language, UiText::Controller).to_string(),
                ),
            ],
            &peripherals.rumble_target,
        ),
    ];
    for system in vm.systems() {
        let sid = system.system_id().to_string();
        rows.push(group_row(
            &format!("advanced.{sid}"),
            system.display_name().to_string(),
            system_field_description(language),
            &format!("system.{sid}"),
        ));
    }
    SectionWire {
        id: "advanced".to_string(),
        label: crate::settings_sections::section_label("advanced", language).to_string(),
        sub: String::new(),
        rows,
        visible: true,
    }
}

/// Audio detail: sample rate and latency for crackle troubleshooting.
/// Reached from Advanced; everyday volume lives on Play.
fn build_audio_detail_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.audio.view.get();
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "audio.detail".to_string(),
        label: t(UiText::AudioDetail),
        sub: t(UiText::SectionAdvanced),
        rows: vec![
            {
                let (choices, selected) = (
                    view.sample_rate_choices
                        .iter()
                        .map(|choice| (choice.value, choice.label.clone()))
                        .collect::<Vec<_>>(),
                    view.sample_rate,
                );
                choice_row(
                    "audio.sample_rate",
                    t(UiText::SampleRate),
                    t(UiText::DescSampleRate),
                    &choices,
                    &selected,
                )
            },
            number_row(
                "audio.latency",
                t(UiText::AudioLatency),
                t(UiText::DescLatency),
                i64::from(view.latency_ms),
                5,
                10,
                200,
                " ms",
            ),
            button_row(
                "reset.audio.detail",
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ),
        ],
        visible: false,
    }
}

fn build_notifications_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.notifications.view.get();
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "notifications".to_string(),
        label: crate::settings_sections::section_label("notifications", language).to_string(),
        sub: String::new(),
        rows: vec![
            header_row(t(UiText::NotificationSectionDisplay)),
            toggle_row(
                "notifications.info_enabled",
                t(UiText::NotificationInfo),
                t(UiText::DescNotifInfo),
                language,
                view.info_enabled,
            ),
            number_row(
                "notifications.info_duration",
                t(UiText::NotificationDuration),
                t(UiText::DescNotifDuration),
                i64::from(view.info_duration_ms),
                500,
                1000,
                30000,
                " ms",
            ),
            choice_row(
                "notifications.info_color",
                t(UiText::NotificationInfo),
                t(UiText::DescNotifColor),
                &view
                    .color_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.info_color,
            ),
            toggle_row(
                "notifications.warning_enabled",
                t(UiText::NotificationWarning),
                t(UiText::DescNotifWarning),
                language,
                view.warning_enabled,
            ),
            number_row(
                "notifications.warning_duration",
                t(UiText::NotificationDuration),
                t(UiText::DescNotifDuration),
                i64::from(view.warning_duration_ms),
                500,
                1000,
                30000,
                " ms",
            ),
            choice_row(
                "notifications.warning_color",
                t(UiText::NotificationWarning),
                t(UiText::DescNotifColor),
                &view
                    .color_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.warning_color,
            ),
            toggle_row(
                "notifications.error_enabled",
                t(UiText::NotificationError),
                t(UiText::DescNotifError),
                language,
                view.error_enabled,
            ),
            toggle_row(
                "notifications.error_modal",
                t(UiText::NotificationErrorModal),
                t(UiText::DescNotifErrorModal),
                language,
                view.error_modal,
            ),
            choice_row(
                "notifications.error_color",
                t(UiText::NotificationError),
                t(UiText::DescNotifColor),
                &view
                    .color_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.error_color,
            ),
            choice_row(
                "notifications.position",
                t(UiText::NotificationPosition),
                t(UiText::DescNotifPosition),
                &view
                    .position_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.position,
            ),
            choice_row(
                "notifications.area_mode",
                t(UiText::NotificationAreaMode),
                t(UiText::DescNotifArea),
                &view
                    .area_mode_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.area_mode,
            ),
            header_row(t(UiText::NotificationSectionStatus)),
            toggle_row(
                "status.enabled",
                t(UiText::NotificationStatus),
                t(UiText::DescStatusDisplay),
                language,
                view.status_enabled,
            ),
            choice_row(
                "status.position",
                t(UiText::NotificationStatusPosition),
                t(UiText::DescStatusPosition),
                &view
                    .position_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.status_position,
            ),
            {
                let mut row = choice_row(
                    "status.area_mode",
                    t(UiText::NotificationStatusAreaMode),
                    t(UiText::DescStatusArea),
                    &view
                        .area_mode_choices
                        .iter()
                        .map(|choice| (choice.value, choice.label.clone()))
                        .collect::<Vec<_>>(),
                    &view.status_area_mode,
                );
                // Same-edge rule: the status area follows the notification
                // area when both share an edge, so the row hides then.
                row.visible = view.position.is_top() != view.status_position.is_top();
                row
            },
            button_row(
                "reset.notifications",
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ),
        ],
        visible: true,
    }
}

/// Controllers: player ports first, then the touch overlay (a global screen
/// UI, never a port device), then global shortcuts (port-independent and
/// always active). The overlay detail holds the size/position/transparency
/// sliders; haptics stays here so there is exactly one place to toggle it.
/// Cartridge pak settings live under Advanced (set-and-forget attic).
fn build_controllers_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
) -> Vec<SectionWire> {
    let overlay = &snapshot.local.touch_overlay;
    let t = |text: UiText| ui_text(language, text).to_string();
    let mut quick_rows = Vec::new();
    for (index, (port, player_id, _, _)) in PLAYER_PORTS.iter().enumerate() {
        let names = port_member_names(port, snapshot, keyboards, language);
        quick_rows.push(group_row(
            &format!("controllers.{player_id}"),
            player_label(index, language),
            port_count_line(language, names.len()),
            player_id,
        ));
        let last = quick_rows.last_mut().expect("player row pushed");
        last.text = port_summary(language, &names);
    }
    let mut visibility_row = choice_row(
        "controls.overlay_visibility",
        t(UiText::TouchOverlay),
        t(UiText::DescOverlayVisibility),
        &[
            (
                TouchOverlayVisibility::Always,
                ui_text(language, UiText::Always).to_string(),
            ),
            (
                TouchOverlayVisibility::Auto,
                ui_text(language, UiText::Auto).to_string(),
            ),
            (
                TouchOverlayVisibility::Hidden,
                ui_text(language, UiText::Hidden).to_string(),
            ),
        ],
        &overlay.visibility,
    );
    if touch_capable == Some(false) {
        // Terminals without a touch screen can never show the overlay:
        // disable Always/Auto with the reason instead of pretending the
        // choice works and hiding at runtime. Order matches the literal
        // above (Always, Auto, Hidden).
        let reason = t(UiText::NoTouchScreen);
        for choice in visibility_row.choices.iter_mut().take(2) {
            choice.enabled = false;
            choice.reason = reason.clone();
        }
    }
    quick_rows.extend([
        header_row(t(UiText::TouchOverlay)),
        visibility_row,
        group_row(
            "controls.overlay_detail_link",
            t(UiText::OverlayDetail),
            t(UiText::DescOverlayDetail),
            "controls.overlay_detail",
        ),
        toggle_row(
            "controls.overlay_haptics",
            t(UiText::OverlayHaptics),
            t(UiText::DescOverlayHaptics),
            language,
            overlay.haptics,
        ),
        header_row(t(UiText::Shortcuts)),
        group_row(
            "controls.shortcuts_link",
            t(UiText::Shortcuts),
            t(UiText::DescShortcutsLink),
            "controls.shortcuts",
        ),
        button_row(
            "reset.controllers",
            t(UiText::ResetScreen),
            t(UiText::DescResetScreen),
        ),
    ]);
    let quick = SectionWire {
        id: "controllers".to_string(),
        label: crate::settings_sections::section_label("controllers", language).to_string(),
        sub: String::new(),
        rows: quick_rows,
        visible: true,
    };
    let overlay_detail = SectionWire {
        id: "controls.overlay_detail".to_string(),
        label: t(UiText::TouchOverlay),
        sub: t(UiText::SectionControllers),
        rows: vec![
            number_row(
                "controls.overlay_opacity",
                t(UiText::OverlayOpacity),
                t(UiText::DescOverlayOpacity),
                i64::from(overlay.opacity_percent),
                1,
                0,
                100,
                "%",
            ),
            number_row(
                "controls.overlay_scale",
                t(UiText::OverlayScale),
                t(UiText::DescOverlayScale),
                i64::from(overlay.scale_percent),
                5,
                50,
                150,
                "%",
            ),
            number_row(
                "controls.overlay_vertical_offset",
                t(UiText::OverlayVerticalOffset),
                t(UiText::DescOverlayOffset),
                i64::from(overlay.vertical_offset_percent),
                1,
                -30,
                30,
                "%",
            ),
            button_row(
                "reset.controls.overlay_detail",
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ),
        ],
        visible: false,
    };
    let shortcuts_view = vm.shortcuts.view.get();
    // Shortcut targets encode a system only as a decoding hint; every
    // global shortcut decodes identically, so the first system suffices.
    let shortcut_system = vm
        .systems()
        .first()
        .map(|system| system.system_id().to_string())
        .unwrap_or_default();
    let shortcuts = SectionWire {
        id: "controls.shortcuts".to_string(),
        label: t(UiText::Shortcuts),
        sub: t(UiText::SectionControllers),
        rows: shortcuts_view
            .rows
            .iter()
            .map(|binding| {
                capture_row(
                    &format!("controls.{}", binding_key(&binding.target)),
                    binding.label.clone(),
                    shortcut_row_description(language),
                    binding_value_text(binding),
                    &encode_target(&binding.target, &shortcut_system),
                )
            })
            .chain(std::iter::once(button_row(
                "reset.controls.shortcuts",
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            )))
            .collect(),
        visible: false,
    };
    vec![quick, overlay_detail, shortcuts]
}

/// One section per machine: its settings fields plus its controller slot
/// assignment. Keyboard bindings and shortcuts need a physical keyboard, so
/// Android shows neither; binding conflicts vanish with them.
/// One section per machine: settings fields plus a navigation row into the
/// machine's controller screen (`input.<system>`). Keyboard bindings live on
/// the controller screen, one drill-down away from the machine settings.
fn build_system_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
    let t = |text: UiText| ui_text(language, text).to_string();
    vm.systems()
        .iter()
        .map(|system| {
            let view = system.view.get();
            let mut rows: Vec<RowWire> = view
                .fields
                .iter()
                .map(|field| {
                    choice_row(
                        &format!("system.{}.{}", view.system_id, field.id.as_str()),
                        field.label.clone(),
                        system_field_description(language),
                        &field
                            .choices
                            .iter()
                            .map(|choice| (choice.value.as_str().to_string(), choice.label.clone()))
                            .collect::<Vec<_>>(),
                        &field.selected.as_str().to_string(),
                    )
                })
                .collect();
            rows.push(group_row(
                &format!("system.{}.controllers", view.system_id),
                ui_text(language, UiText::Controller).to_string(),
                ui_text(language, UiText::DescSystemControllers).to_string(),
                &format!("input.{}", view.system_id),
            ));
            rows.push(button_row(
                &format!("reset.system.{}", view.system_id),
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ));
            SectionWire {
                id: format!("system.{}", view.system_id),
                label: view.label.clone(),
                sub: t(UiText::SectionAdvanced),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// One controller screen per machine: the emulated controller type per
/// slot (IA "controller type is per machine port"). Hidden from the
/// category list; reached from the player type screens and from the
/// machine screen under Advanced. Keyboard bindings moved to `keys.*`.
fn build_input_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
    let t = |text: UiText| ui_text(language, text).to_string();
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            let mut rows = slot_rows(&view, language);
            rows.push(button_row(
                &format!("reset.input.{}", view.system_id),
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ));
            SectionWire {
                id: format!("input.{}", view.system_id),
                label: view.label.clone(),
                sub: t(UiText::ControllerType),
                rows,
                visible: false,
            }
        })
        .collect()
}

/// Slot assignment rows shared by the machine controller screens.
fn slot_rows(
    view: &nerust_gui_viewmodel::settings::dto::InputTabView,
    language: AppLanguage,
) -> Vec<RowWire> {
    // Profiles taken by another slot cannot be picked here; keep the
    // owner slot label so Kotlin can explain the disabled choice.
    let taken: std::collections::HashMap<String, String> = view
        .slots
        .iter()
        .filter_map(|slot| {
            slot.selected_profile_id
                .clone()
                .map(|profile| (profile, slot.slot_id.as_str().to_string()))
        })
        .collect();
    let slot_labels: std::collections::HashMap<String, String> = view
        .slots
        .iter()
        .map(|slot| (slot.slot_id.as_str().to_string(), slot.label.clone()))
        .collect();
    view.slots
        .iter()
        .map(|slot| {
            let mut row = choice_row(
                &format!("system.{}.slot.{}", view.system_id, slot.slot_id.as_str()),
                slot.label.clone(),
                slot_description(language),
                &slot
                    .choices
                    .iter()
                    .map(|choice| {
                        (
                            serde_json::to_value(&choice.value).unwrap_or(serde_json::Value::Null),
                            choice.label.clone(),
                        )
                    })
                    .collect::<Vec<_>>(),
                &serde_json::to_value(&slot.selected_profile_id).unwrap_or(serde_json::Value::Null),
            );
            // A multi-port profile (NES famicom set) already covers this
            // slot from another one: nothing but "not assigned" is
            // pickable here until that slot releases it. The desktop
            // switch disables the whole row; the wire has no row switch,
            // so every concrete choice carries the owner instead.
            let covering_owner = slot.occupied_by_other_slot.then(|| {
                view.slots
                    .iter()
                    .find(|other| {
                        other.slot_id != slot.slot_id && other.selected_profile_id.is_some()
                    })
                    .map(|other| other.label.clone())
                    .unwrap_or_default()
            });
            for (wire, choice) in row.choices.iter_mut().zip(slot.choices.iter()) {
                let owner = match &choice.value {
                    None => None,
                    Some(profile) => taken
                        .get(profile)
                        .filter(|owner| *owner != slot.slot_id.as_str()),
                };
                let covered = covering_owner.as_ref().filter(|_| choice.value.is_some());
                wire.enabled = owner.is_none() && covered.is_none();
                wire.reason = owner
                    .and_then(|owner| slot_labels.get(owner))
                    .cloned()
                    .or_else(|| covered.cloned())
                    .unwrap_or_default();
            }
            row
        })
        .collect()
}

/// Player screens (one per global port): the Devices row opens the device
/// picker, the Controller-type row opens the per-machine type list.
/// Summaries read from the port device set, never from machine sections.
fn build_player_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
    keyboards: &[KeyboardInfo],
) -> Vec<SectionWire> {
    let t = |text: UiText| ui_text(language, text).to_string();
    let mut sections = Vec::new();
    for (index, (port, player_id, devices_id, types_id)) in PLAYER_PORTS.iter().enumerate() {
        let label = player_label(index, language);
        let names = port_member_names(port, snapshot, keyboards, language);
        sections.push(SectionWire {
            id: player_id.to_string(),
            label: label.clone(),
            sub: t(UiText::SectionControllers),
            rows: vec![
                {
                    let mut row = group_row(
                        &format!("{player_id}.devices"),
                        t(UiText::Devices),
                        t(UiText::FollowsDevice),
                        devices_id,
                    );
                    row.text = port_summary(language, &names);
                    row
                },
                group_row(
                    &format!("{player_id}.type"),
                    t(UiText::ControllerType),
                    system_field_description(language),
                    types_id,
                ),
            ],
            visible: false,
        });
        // Controller-type list: one group row per machine that actually
        // has a slot on this port (NES-only for player 2). Slot ids must
        // end in player1/player2; fail loudly in tests when a factory
        // diverges instead of silently dropping its slots.
        debug_assert!(
            {
                let mut ok = true;
                for input in vm.inputs() {
                    for slot in &input.view.get().slots {
                        ok &= slot_port_index(slot.slot_id.as_str()).is_some();
                    }
                }
                ok
            },
            "slot ids must end with player1/player2"
        );
        let type_rows: Vec<RowWire> = vm
            .inputs()
            .iter()
            .filter(|input| {
                input
                    .view
                    .get()
                    .slots
                    .iter()
                    .any(|slot| slot_port_index(slot.slot_id.as_str()) == Some(index))
            })
            .map(|input| {
                let view = input.view.get();
                group_row(
                    &format!("{types_id}.{}", view.system_id),
                    view.label.clone(),
                    type_profile_summary(&view, index, language),
                    &format!("input.{}", view.system_id),
                )
            })
            .collect();
        sections.push(SectionWire {
            id: types_id.to_string(),
            label: t(UiText::ControllerType),
            sub: label.clone(),
            rows: type_rows,
            visible: false,
        });
        sections.push(build_devices_section(
            port, devices_id, &label, snapshot, keyboards, language,
        ));
    }
    sections.extend(build_device_sections(vm, keyboards, language));
    sections
}

/// Slot id suffix rule: every factory names player slots
/// `<system>.attachment.playerN`. Player 2 exists on NES only.
fn slot_port_index(slot_id: &str) -> Option<usize> {
    if slot_id.ends_with("player2") {
        Some(1)
    } else if slot_id.ends_with("player1") {
        Some(0)
    } else {
        None
    }
}

/// Current controller-type summary for one machine on one port: the
/// selected profile label, or Not assigned when the slot is empty.
fn type_profile_summary(
    view: &nerust_gui_viewmodel::settings::dto::InputTabView,
    port_index: usize,
    language: AppLanguage,
) -> String {
    view.slots
        .iter()
        .find(|slot| slot_port_index(slot.slot_id.as_str()) == Some(port_index))
        .and_then(|slot| {
            slot.selected_profile_id.as_ref().and_then(|selected| {
                slot.choices.iter().find_map(|choice| {
                    (choice.value.as_deref() == Some(selected.as_str()))
                        .then(|| choice.label.clone())
                })
            })
        })
        .unwrap_or_else(|| ui_text(language, UiText::NotAssigned).to_string())
}

/// Display names for a port's member ids in set order. Unknown members
/// fall back to a generic label so a stale id never renders blank.
fn port_member_names(
    port: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    language: AppLanguage,
) -> Vec<String> {
    // Effective members only: the built-in row renders solely while the
    // terminal actually enumerates it. Otherwise phones and tablets
    // without a hardware keyboard would show a phantom "Built-in
    // Keyboard" from the shared default. Unplugged USB descriptors stay
    // listed (reconnect restores them); gamepads always count.
    let builtin_present = keyboards
        .iter()
        .any(|info| info.member == BUILTIN_KEYBOARD_MEMBER);
    snapshot
        .app_state
        .port_devices
        .get(port)
        .map(|members| {
            members
                .iter()
                .filter(|member| member.as_str() != BUILTIN_KEYBOARD_MEMBER || builtin_present)
                .map(|member| keyboard_member_name(member, keyboards, language))
                .collect()
        })
        .unwrap_or_default()
}

/// Drop the built-in membership when fresh enumeration reports no
/// built-in keyboard: a capability flag, not a known device, so there is
/// nothing to restore on reconnect. USB descriptors are never pruned
/// here — unplug/replug cycles keep their assignment.
fn prune_missing_builtin(snapshot: &mut SettingsSnapshot, keyboards: &[KeyboardInfo]) {
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

fn keyboard_member_name(member: &str, keyboards: &[KeyboardInfo], language: AppLanguage) -> String {
    let t = |text: UiText| ui_text(language, text).to_string();
    if member == BUILTIN_KEYBOARD_MEMBER {
        t(UiText::BuiltInKeyboard)
    } else if let Some(info) = keyboards.iter().find(|info| info.member == member) {
        if info.name.trim().is_empty() {
            // OS gave no name: fall back instead of rendering blank.
            t(UiText::ExternalKeyboard)
        } else {
            info.name.clone()
        }
    } else if member.starts_with(KEYBOARD_MEMBER_PREFIX) {
        t(UiText::ExternalKeyboard)
    } else if member.starts_with(GAMEPAD_MEMBER_PREFIX) {
        t(UiText::GamepadDevice)
    } else {
        member.to_string()
    }
}

/// Device picker: the dynamic keyboard enumeration (built-in row when
/// present, one row per external descriptor, BT/USB agnostic, virtual
/// devices excluded — classified Kotlin-side). Row tap toggles port
/// membership; Details opens the device screen. Gamepad rows appear here
/// once the gamepad backend lands; the gamepad group is omitted until then.
fn build_devices_section(
    port: &str,
    devices_id: &str,
    player: &str,
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    language: AppLanguage,
) -> SectionWire {
    let t = |text: UiText| ui_text(language, text).to_string();
    let members = snapshot.app_state.port_devices.get(port);
    let is_member = |member: &str| {
        members
            .map(|set| set.iter().any(|entry| entry == member))
            .unwrap_or(false)
    };
    let mut rows = vec![header_row(t(UiText::Keyboard))];
    if keyboards.is_empty() {
        rows.push(info_row(
            &format!("{devices_id}.none"),
            t(UiText::NoKeyboards),
            t(UiText::DescNoKeyboards),
            String::new(),
        ));
    }
    for info in keyboards {
        let mut row = toggle_row(
            &format!("devices.{port}.{}", info.member),
            keyboard_member_name(&info.member, keyboards, language),
            if info.builtin {
                t(UiText::AlwaysAvailable)
            } else {
                t(UiText::Connected)
            },
            language,
            is_member(&info.member),
        );
        // The opaque member id rides the target; the port comes from the key.
        row.target = info.member.clone();
        row.detail_target = format!("device.{}", info.member);
        rows.push(row);
    }
    rows.push(button_row(
        &format!("reset.{devices_id}"),
        t(UiText::ResetScreen),
        t(UiText::DescResetScreen),
    ));
    SectionWire {
        id: devices_id.to_string(),
        label: t(UiText::Devices),
        sub: player.to_string(),
        rows,
        visible: false,
    }
}

/// One device screen per enumerated keyboard: connection/capability
/// readouts (labels are self-evident, so no descriptions) plus the
/// per-system key-mapping tables (shared single table, not per device).
fn build_device_sections(
    vm: &SettingsViewModel,
    keyboards: &[KeyboardInfo],
    language: AppLanguage,
) -> Vec<SectionWire> {
    let t = |text: UiText| ui_text(language, text).to_string();
    keyboards
        .iter()
        .map(|info| {
            let mut rows = vec![
                info_row(
                    &format!("device.{}.connection", info.member),
                    t(UiText::Connection),
                    String::new(),
                    if info.builtin {
                        t(UiText::AlwaysAvailable)
                    } else {
                        t(UiText::Connected)
                    },
                ),
                info_row(
                    &format!("device.{}.capabilities", info.member),
                    t(UiText::Capabilities),
                    String::new(),
                    t(UiText::SharesAcrossPlayers),
                ),
                header_row(t(UiText::KeyMapping)),
            ];
            for system in vm.systems() {
                let sid = system.system_id().to_string();
                rows.push(group_row(
                    &format!("device.{}.keys.{sid}", info.member),
                    system.display_name().to_string(),
                    t(UiText::KeyMappingDesc),
                    &format!("keys.{sid}"),
                ));
            }
            SectionWire {
                id: format!("device.{}", info.member),
                label: if info.builtin {
                    t(UiText::BuiltInKeyboard)
                } else if info.name.trim().is_empty() {
                    t(UiText::ExternalKeyboard)
                } else {
                    info.name.clone()
                },
                sub: t(UiText::Devices),
                rows,
                visible: false,
            }
        })
        .collect()
}
/// One key-mapping screen per machine: the shared single keyboard table
/// (per-device split is future work). Reached from each keyboard device
/// detail screen; the controller screens hold slots only.
fn build_keys_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
    let t = |text: UiText| ui_text(language, text).to_string();
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            let mut rows: Vec<RowWire> = Vec::new();
            for section in &view.sections {
                rows.push(header_row(section.label.clone()));
                rows.extend(section.rows.iter().map(|binding| {
                    capture_row(
                        &format!(
                            "keys.{}.binding.{}",
                            view.system_id,
                            binding_key(&binding.target)
                        ),
                        binding.label.clone(),
                        String::new(),
                        binding_value_text(binding),
                        &encode_target(&binding.target, &view.system_id.to_string()),
                    )
                }));
            }
            rows.push(button_row(
                &format!("reset.keys.{}", view.system_id),
                t(UiText::ResetScreen),
                t(UiText::DescResetScreen),
            ));
            SectionWire {
                id: format!("keys.{}", view.system_id),
                label: view.label.clone(),
                sub: t(UiText::KeyMapping),
                rows,
                visible: false,
            }
        })
        .collect()
}

fn binding_key(target: &CaptureTarget) -> String {
    match target {
        CaptureTarget::Binding {
            attachment,
            control,
            ..
        } => format!("{attachment}.{control}"),
        CaptureTarget::Shortcut(action) => format!("shortcut.{action:?}"),
    }
}

fn binding_value_text(binding: &nerust_gui_viewmodel::settings::dto::BindingRowView) -> String {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    match &binding.value {
        BindingValueView::Unbound(label)
        | BindingValueView::Bound(label)
        | BindingValueView::Capturing(label) => label.clone(),
    }
}

/// Opaque capture target: newline-free JSON the key must not parse.
fn encode_target(target: &CaptureTarget, system: &str) -> String {
    match target {
        CaptureTarget::Binding {
            attachment,
            control,
            ..
        } => serde_json::json!({
            "kind": "binding",
            "system": system,
            "attachment": attachment,
            "control": control,
        })
        .to_string(),
        CaptureTarget::Shortcut(action) => {
            serde_json::json!({"kind": "shortcut", "action": action}).to_string()
        }
    }
}

fn decode_target(encoded: &str, deps: &VmDeps) -> Result<CaptureTarget, String> {
    let value: serde_json::Value =
        serde_json::from_str(encoded).map_err(|error| error.to_string())?;
    match value.get("kind").and_then(|kind| kind.as_str()) {
        Some("binding") => {
            let system = value
                .get("system")
                .and_then(|system| system.as_str())
                .ok_or_else(|| "capture target misses system".to_string())?;
            let factory = deps
                .factories
                .iter()
                .find(|factory| factory.system_id().to_string() == system)
                .ok_or_else(|| format!("unknown system: {system}"))?;
            Ok(CaptureTarget::Binding {
                system: factory.system_id(),
                attachment: value
                    .get("attachment")
                    .and_then(|attachment| attachment.as_str())
                    .unwrap_or_default()
                    .to_string(),
                control: value
                    .get("control")
                    .and_then(|control| control.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        Some("shortcut") => {
            let action: nerust_settings_core::model::input::ShortcutAction =
                serde_json::from_value(value.get("action").cloned().unwrap_or_default())
                    .map_err(|error| error.to_string())?;
            Ok(CaptureTarget::Shortcut(action))
        }
        _ => Err("unknown capture target".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use nerust_gba_factory::GbaFactory;
    use nerust_gba_settings::GbaSettings;
    use nerust_gbc_factory::GbcFactory;
    use nerust_gbc_settings::GbcSettings;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_nes_factory::NesFactory;
    use nerust_nes_settings::{NesSettings, NesVideoFilter};
    use nerust_settings_core::model::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };
    use nerust_settings_traits::SystemSettings;

    fn default_snapshot() -> SettingsSnapshot {
        let mut shared = DesktopSharedSettings::default();
        shared.systems.insert(
            NesFactory.system_id(),
            Box::new(NesSettings::default()) as Box<dyn SystemSettings>,
        );
        shared.systems.insert(
            GbcFactory.system_id(),
            Box::new(GbcSettings::default()) as Box<dyn SystemSettings>,
        );
        shared.systems.insert(
            GbaFactory.system_id(),
            Box::new(GbaSettings::default()) as Box<dyn SystemSettings>,
        );
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn registry() -> SystemRegistry {
        SystemRegistry::new(vec![
            Arc::new(NesFactory),
            Arc::new(GbcFactory),
            Arc::new(GbaFactory),
        ])
    }

    fn test_deps() -> VmDeps {
        let registry = registry();
        VmDeps {
            factories: registry.all().to_vec(),
            rates: Arc::from([44_100u32, 48_000].as_slice()),
        }
    }

    fn with_test_deps<T>(operation: impl FnOnce() -> T) -> T {
        // Never reset: tests share the process-wide slot and run in parallel.
        // Installs are equivalent, so a concurrent overwrite is benign.
        install_vm_deps(test_deps().factories, test_deps().rates);
        operation()
    }

    fn test_keyboards() -> Vec<KeyboardInfo> {
        vec![
            KeyboardInfo {
                member: "kbd:builtin".to_string(),
                name: "Built-in Keyboard".to_string(),
                builtin: true,
            },
            KeyboardInfo {
                member: "kbd:usb-1".to_string(),
                name: "USB Keyboard".to_string(),
                builtin: false,
            },
        ]
    }

    fn test_payload() -> (SettingsSnapshot, ViewPayload) {
        test_payload_with_keyboards(&test_keyboards())
    }

    fn test_payload_with_keyboards(keyboards: &[KeyboardInfo]) -> (SettingsSnapshot, ViewPayload) {
        with_test_deps(|| {
            let snapshot = default_snapshot();
            let json = open_payload(&snapshot, keyboards, None).expect("open payload builds");
            let payload: ViewPayload = serde_json::from_str(&json).expect("payload parses");
            assert_eq!(payload.schema_version, SETTINGS_SCHEMA_VERSION);
            (snapshot, payload)
        })
    }

    fn row<'a>(payload: &'a ViewPayload, key: &str) -> &'a RowWire {
        payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.key == key)
            .unwrap_or_else(|| panic!("row {key} exists"))
    }

    fn apply(
        snapshot: &mut SettingsSnapshot,
        capture: &mut CaptureSlot,
        keyboards: &mut Vec<KeyboardInfo>,
        key: &str,
        op: &str,
        index: usize,
        number: i64,
    ) -> ViewPayload {
        with_test_deps(|| {
            let command_json = serde_json::to_string(&WireCommand {
                key: key.to_string(),
                index,
                number,
                op: op.to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            let out = apply_command_to(snapshot, &command_json, capture, keyboards, &mut None)
                .unwrap_or_else(|error| panic!("command failed: {error}"));
            serde_json::from_str(&out).unwrap()
        })
    }

    fn set(snapshot: &mut SettingsSnapshot, key: &str, index: usize, number: i64) -> ViewPayload {
        apply(
            snapshot,
            &mut None,
            &mut test_keyboards(),
            key,
            "set",
            index,
            number,
        )
    }

    #[test]
    fn open_payload_covers_legacy_items() {
        let (_snapshot, payload) = test_payload();
        // Every row the old descriptor dialog exposed must exist.
        for key in [
            "audio.muted",
            "audio.volume",
            "audio.latency",
            "audio.sample_rate",
            "video.vsync",
            "video.orientation",
            "general.storage_policy",
            "controls.overlay_visibility",
            "controls.overlay_opacity",
            "controls.overlay_scale",
            "controls.overlay_vertical_offset",
            "controls.overlay_haptics",
            "controls.cartridge_motion",
            "controls.cartridge_rumble",
            "controls.cartridge_rumble_strength",
            "controls.cartridge_rumble_target",
            "notifications.info_enabled",
            "notifications.warning_enabled",
            "notifications.error_enabled",
            "notifications.position",
            "notifications.area_mode",
            "status.enabled",
            "status.position",
            "status.area_mode",
        ] {
            row(&payload, key);
        }
        assert!(
            payload
                .sections
                .iter()
                .flat_map(|section| &section.rows)
                .any(|row| row.key.starts_with("system.") && row.key.ends_with("video.filter")),
            "system filter rows exist"
        );
    }

    #[test]
    fn zero_based_top_level_hides_machine_names() {
        let (_snapshot, payload) = test_payload();
        // Top-level taxonomy is purpose-based: no machine names on top.
        let visible: Vec<&str> = payload
            .sections
            .iter()
            .filter(|section| section.visible)
            .map(|section| section.id.as_str())
            .collect();
        assert_eq!(
            visible,
            vec![
                "play",
                "controllers",
                "saves",
                "notifications",
                "app",
                "advanced"
            ],
            "visible categories are purpose-based"
        );
        for id in ["system.nes", "system.gbc", "system.gba"] {
            let machine = payload
                .sections
                .iter()
                .find(|section| section.id == id)
                .unwrap_or_else(|| panic!("{id} exists"));
            assert!(!machine.visible, "{id} hides under Advanced");
            assert!(
                machine
                    .rows
                    .iter()
                    .any(|row| row.kind == super::RowKind::Choice),
                "{id} keeps its fields"
            );
            assert!(
                machine
                    .rows
                    .iter()
                    .any(|row| row.kind == super::RowKind::Group
                        && row.target == format!("input.{}", id.trim_start_matches("system."))),
                "{id} links to its controller screen"
            );
        }
    }

    #[test]
    fn machine_sections_link_to_hidden_controller_screens() {
        let (_snapshot, payload) = test_payload();
        // Controller screens hold slots only; bindings moved to keys.*.
        let input = payload
            .sections
            .iter()
            .find(|section| section.id == "input.nes")
            .expect("controller screen exists");
        assert!(!input.visible, "controller screens hide from categories");
        assert!(
            input
                .rows
                .iter()
                .any(|row| row.key.starts_with("system.nes.slot.")),
            "slots live on the controller screen"
        );
        assert!(
            input
                .rows
                .iter()
                .all(|row| row.kind != super::RowKind::Capture),
            "bindings moved out of the controller screen"
        );
        let keys = payload
            .sections
            .iter()
            .find(|section| section.id == "keys.nes")
            .expect("key mapping screen exists");
        assert!(!keys.visible, "key mapping screens hide from categories");
        assert!(
            keys.rows
                .iter()
                .any(|row| row.kind == super::RowKind::Capture),
            "keyboard bindings are capturable rows"
        );
        assert!(
            payload
                .sections
                .iter()
                .all(|section| section.id != "shortcuts"),
            "no top-level shortcuts section on Android"
        );
        // Global shortcuts live one drill-down under Controllers, next to the
        // overlay detail screen: port-independent and always active.
        for (id, probe_key) in [
            ("controls.overlay_detail", "controls.overlay_opacity"),
            ("controls.shortcuts", "controls.shortcut.TogglePause"),
        ] {
            let hidden = payload
                .sections
                .iter()
                .find(|section| section.id == id)
                .unwrap_or_else(|| panic!("{id} screen exists"));
            assert!(!hidden.visible, "{id} hides from categories");
            assert!(
                hidden.rows.iter().any(|row| row.key == probe_key),
                "{id} holds its rows"
            );
            assert!(
                payload
                    .sections
                    .iter()
                    .flat_map(|section| &section.rows)
                    .any(|row| row.kind == super::RowKind::Group && row.target == id),
                "controllers links to {id}"
            );
        }
        let shortcuts = payload
            .sections
            .iter()
            .find(|section| section.id == "controls.shortcuts")
            .expect("shortcuts screen exists");
        assert_eq!(
            shortcuts.rows.len(),
            9,
            "all global shortcut actions plus the screen reset stay reachable"
        );
        assert!(
            shortcuts
                .rows
                .iter()
                .filter(|row| row.kind == super::RowKind::Capture)
                .count()
                == 8,
            "shortcuts rows are capturable"
        );
    }

    #[test]
    fn player_screens_wire_devices_types_and_summaries() {
        let (_snapshot, payload) = test_payload();
        // Default draft: built-in keyboard drives player 1.
        for (player_id, devices_id, types_id) in [
            ("player.1", "devices.1", "types.1"),
            ("player.2", "devices.2", "types.2"),
        ] {
            let player = payload
                .sections
                .iter()
                .find(|section| section.id == *player_id)
                .unwrap_or_else(|| panic!("{player_id} exists"));
            assert!(!player.visible, "{player_id} hides from categories");
            let devices_row = player
                .rows
                .iter()
                .find(|row| row.target == *devices_id)
                .expect("devices row exists");
            assert!(
                !devices_row.description.is_empty(),
                "devices row explains itself"
            );
            let types_row = player
                .rows
                .iter()
                .find(|row| row.target == *types_id)
                .expect("controller type row exists");
            assert_eq!(types_row.text, "", "type row is a bare chevron");
            // Player rows on the controllers screen carry the same summary.
            let parent_row = payload
                .sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|row| row.target == *player_id)
                .expect("controllers links to the player screen");
            assert_eq!(
                parent_row.text, devices_row.text,
                "player summary matches the devices value"
            );
        }
        // Player 1 shows its built-in keyboard; player 2 is unassigned.
        let p1 = row(&payload, "controllers.player.1");
        assert!(
            p1.text.contains("Built-in Keyboard"),
            "p1 summary names the device"
        );
        assert_eq!(row(&payload, "controllers.player.2").text, "Not assigned");
        // Types screens fan out per machine with a player-1 row everywhere
        // and a player-2 row on NES only.
        assert_eq!(
            payload
                .sections
                .iter()
                .find(|section| section.id == "types.1")
                .expect("types.1 exists")
                .rows
                .len(),
            3,
            "every machine has a player-1 slot"
        );
        assert_eq!(
            payload
                .sections
                .iter()
                .find(|section| section.id == "types.2")
                .expect("types.2 exists")
                .rows
                .len(),
            1,
            "only NES has a player-2 slot"
        );
    }

    #[test]
    fn devices_screen_lists_keyboards_with_membership_switches() {
        let (_snapshot, payload) = test_payload_with_keyboards(&[]);
        let devices = payload
            .sections
            .iter()
            .find(|section| section.id == "devices.1")
            .expect("devices screen exists");
        assert!(
            devices.rows.iter().any(|row| row.key == "devices.1.none"),
            "empty enumeration explains itself instead of rendering blank"
        );
        let (_snapshot, payload) = test_payload();
        let devices = payload
            .sections
            .iter()
            .find(|section| section.id == "devices.1")
            .expect("devices screen exists");
        let builtin = devices
            .rows
            .iter()
            .find(|row| row.key == "devices.p1.kbd:builtin")
            .expect("built-in keyboard row exists");
        assert_eq!(builtin.kind, super::RowKind::Toggle);
        assert_eq!(
            builtin.selected, 1,
            "built-in keyboard drives p1 by default"
        );
        assert_eq!(builtin.detail_target, "device.kbd:builtin");
        let usb = devices
            .rows
            .iter()
            .find(|row| row.key == "devices.p1.kbd:usb-1")
            .expect("external keyboard row exists");
        assert_eq!(usb.selected, 0, "connected keyboards start unassigned");
        // Every enumerated keyboard gets a detail screen with status plus
        // per-system key mapping links.
        for member in ["kbd:builtin", "kbd:usb-1"] {
            let detail = payload
                .sections
                .iter()
                .find(|section| section.id == format!("device.{member}"))
                .unwrap_or_else(|| panic!("device screen for {member} exists"));
            assert!(!detail.visible, "device screens hide from categories");
            for system in ["nes", "gbc", "gba"] {
                assert!(
                    detail
                        .rows
                        .iter()
                        .any(|row| row.kind == super::RowKind::Group
                            && row.target == format!("keys.{system}")),
                    "{member} links to the {system} key table"
                );
            }
        }
    }

    #[test]
    fn device_toggle_flips_port_membership() {
        let (mut snapshot, _) = test_payload();
        let mut keyboards = test_keyboards();
        // USB keyboard joins player 2: switch on.
        let on = apply(
            &mut snapshot,
            &mut None,
            &mut keyboards,
            "devices.p2.kbd:usb-1",
            "set",
            1,
            0,
        );
        assert_eq!(
            row(&on, "devices.p2.kbd:usb-1").selected,
            1,
            "switch reflects membership"
        );
        assert!(
            snapshot
                .app_state
                .port_devices
                .get("p2")
                .is_some_and(|set| set.iter().any(|member| member == "kbd:usb-1")),
            "membership persists in the draft"
        );
        assert!(
            row(&on, "controllers.player.2")
                .text
                .contains("USB Keyboard"),
            "player summary follows membership"
        );
        // And leaves again: switch off prunes the member id.
        let off = apply(
            &mut snapshot,
            &mut None,
            &mut keyboards,
            "devices.p2.kbd:usb-1",
            "set",
            0,
            0,
        );
        assert_eq!(
            row(&off, "controllers.player.2").text,
            "Not assigned",
            "empty port reads Not assigned"
        );
    }

    #[test]
    fn phantom_builtin_never_surfaces_without_hardware() {
        // Phones and tablets enumerate no built-in keyboard, yet the
        // shared default drafts p1=[kbd:builtin]. Summaries and counts
        // must read empty, and the first fresh enumeration prunes it.
        let (mut snapshot, _) = test_payload_with_keyboards(&[]);
        assert_eq!(
            row(&test_payload_with_keyboards(&[]).1, "controllers.player.1").text,
            "Not assigned",
            "no hardware, no phantom controller name"
        );
        let mut keyboards = Vec::new();
        let command_json = serde_json::to_string(&WireCommand {
            key: "keyboards".to_string(),
            index: 0,
            number: 0,
            op: "keyboards".to_string(),
            payload: serde_json::to_value(vec![KeyboardInfo {
                member: "kbd:usb-1".to_string(),
                name: "USB Keyboard".to_string(),
                builtin: false,
            }])
            .unwrap(),
        })
        .unwrap();
        with_test_deps(|| {
            apply_command_to(
                &mut snapshot,
                &command_json,
                &mut None,
                &mut keyboards,
                &mut None,
            )
            .expect("keyboards op applies")
        });
        assert!(
            snapshot
                .app_state
                .port_devices
                .get("p1")
                .is_none_or(|set| !set.iter().any(|member| member == "kbd:builtin")),
            "builtin membership pruned without hardware"
        );
        // …while an unplugged USB descriptor keeps its assignment so a
        // reconnect restores it without re-setup.
        snapshot
            .app_state
            .port_devices
            .entry("p1".to_string())
            .or_default()
            .push("kbd:usb-1".to_string());
        let off = with_test_deps(|| {
            apply_command_to(
                &mut snapshot,
                &serde_json::to_string(&WireCommand {
                    key: "keyboards".to_string(),
                    index: 0,
                    number: 0,
                    op: "keyboards".to_string(),
                    payload: serde_json::Value::Array(vec![]),
                })
                .unwrap(),
                &mut None,
                &mut Vec::new(),
                &mut None,
            )
            .expect("empty enumeration applies")
        });
        let payload: ViewPayload = serde_json::from_str(&off).unwrap();
        assert!(
            snapshot
                .app_state
                .port_devices
                .get("p1")
                .is_some_and(|set| set.iter().any(|member| member == "kbd:usb-1")),
            "USB assignment survives unplug"
        );
        assert_eq!(
            row(&payload, "controllers.player.1").text,
            "External keyboard",
            "unplugged USB still names its fallback"
        );
    }

    #[test]
    fn keyboards_op_upserts_enumeration_without_mutating_draft() {
        let (mut snapshot, _) = test_payload_with_keyboards(&[]);
        let before = snapshot.clone();
        let mut keyboards = Vec::new();
        let command_json = serde_json::to_string(&WireCommand {
            key: "keyboards".to_string(),
            index: 0,
            number: 0,
            op: "keyboards".to_string(),
            payload: serde_json::to_value(test_keyboards()).unwrap(),
        })
        .unwrap();
        let out = with_test_deps(|| {
            apply_command_to(
                &mut snapshot,
                &command_json,
                &mut None,
                &mut keyboards,
                &mut None,
            )
            .expect("keyboards op applies")
        });
        assert_eq!(snapshot, before, "enumeration never touches the draft");
        assert_eq!(keyboards.len(), 2, "vault holds the enumerated keyboards");
        let payload: ViewPayload = serde_json::from_str(&out).unwrap();
        assert!(
            payload
                .sections
                .iter()
                .flat_map(|section| &section.rows)
                .any(|row| row.key == "devices.p1.kbd:usb-1"),
            "refreshed payload lists the enumerated keyboard"
        );
    }

    fn touch_payload(capable: Option<bool>) -> ViewPayload {
        let (mut snapshot, _) = test_payload();
        if let Some(value) = capable {
            let command_json = serde_json::to_string(&WireCommand {
                key: "touch".to_string(),
                index: 0,
                number: 0,
                op: "touch".to_string(),
                payload: serde_json::json!({ "capable": value }),
            })
            .unwrap();
            let mut touch = None;
            let out = with_test_deps(|| {
                apply_command_to(
                    &mut snapshot,
                    &command_json,
                    &mut None,
                    &mut test_keyboards(),
                    &mut touch,
                )
                .expect("touch op applies")
            });
            assert_eq!(touch, capable, "vault records touch capability");
            return serde_json::from_str(&out).unwrap();
        }
        // Unknown capability (old client, unit default): no push arrives,
        // rows behave as before.
        let touch = None;
        with_test_deps(|| {
            open_payload(&snapshot, &test_keyboards(), touch)
                .map(|json| serde_json::from_str(&json).unwrap())
                .expect("open payload builds")
        })
    }

    #[test]
    fn touchless_terminal_disables_overlay_choices() {
        let payload = touch_payload(Some(false));
        let visibility = row(&payload, "controls.overlay_visibility");
        assert_eq!(
            visibility.choices.len(),
            3,
            "Always/Auto/Hidden stay listed"
        );
        for choice in visibility.choices.iter().take(2) {
            assert!(!choice.enabled, "Always/Auto are not enableable");
            assert!(
                !choice.reason.is_empty(),
                "disabled choice explains the missing touch screen"
            );
        }
        assert!(
            visibility.choices[2].enabled,
            "Hidden stays selectable as the only honest value"
        );
    }

    #[test]
    fn touch_capable_terminal_keeps_overlay_choices() {
        for capable in [None, Some(true)] {
            let payload = touch_payload(capable);
            let visibility = row(&payload, "controls.overlay_visibility");
            assert!(
                visibility.choices.iter().all(|choice| choice.enabled),
                "unknown/touch terminals keep every overlay choice"
            );
        }
    }

    #[test]
    fn reset_section_restores_screen_defaults() {
        let (mut snapshot, _) = test_payload();
        let mut keyboards = test_keyboards();
        // Dirty the volume, then reset the play screen.
        set(&mut snapshot, "audio.volume", 0, 77);
        assert_eq!(snapshot.local.audio.master_volume_percent, 77);
        let reset = serde_json::to_string(&WireCommand {
            key: "reset.play".to_string(),
            index: 0,
            number: 0,
            op: "reset_section".to_string(),
            payload: serde_json::Value::Null,
        })
        .unwrap();
        let out = with_test_deps(|| {
            apply_command_to(&mut snapshot, &reset, &mut None, &mut keyboards, &mut None)
                .expect("reset applies")
        });
        let payload: ViewPayload = serde_json::from_str(&out).unwrap();
        assert_eq!(
            row(&payload, "audio.volume").number,
            i64::from(
                HostBackendLocalSettings::default()
                    .audio
                    .master_volume_percent
            ),
            "volume returns to default"
        );
    }
    #[test]
    fn taken_profiles_disable_in_other_slots() {
        let (mut snapshot, payload) = test_payload();
        // Give player1 its first concrete profile, then player2 must refuse it.
        let p1_key = "system.nes.slot.nes.attachment.player1";
        let p2_key = "system.nes.slot.nes.attachment.player2";
        let p1_choices = &row(&payload, p1_key).choices;
        let p2_values: Vec<_> = row(&payload, p2_key)
            .choices
            .iter()
            .map(|choice| choice.value.clone())
            .collect();
        let (first, profile) = p1_choices
            .iter()
            .enumerate()
            .find(|(_, choice)| !choice.value.is_null() && p2_values.contains(&choice.value))
            .map(|(index, choice)| (index, choice.value.clone()))
            .expect("shared concrete profile exists");
        let next = set(&mut snapshot, p1_key, first, 0);
        let p2 = row(&next, p2_key);
        let taken = p2
            .choices
            .iter()
            .find(|choice| choice.value == profile)
            .expect("same profile listed for player2");
        assert!(!taken.enabled, "taken profile disables elsewhere");
        assert!(
            !taken.reason.is_empty(),
            "disabled choice names its owner slot"
        );
        // ...but stays enabled in its own slot.
        let own = row(&next, p1_key)
            .choices
            .iter()
            .find(|choice| choice.value == profile)
            .expect("profile listed for player1");
        assert!(own.enabled);
    }

    #[test]
    fn covered_slot_disables_concrete_controllers() {
        let (mut snapshot, payload) = test_payload();
        let p1_key = "system.nes.slot.nes.attachment.player1";
        let p2_key = "system.nes.slot.nes.attachment.player2";
        // The famicom set physically covers both slots; P2 then offers
        // nothing but "not assigned" until P1 releases it.
        let famicom = row(&payload, p1_key)
            .choices
            .iter()
            .find(|choice| {
                choice
                    .value
                    .as_str()
                    .is_some_and(|value| value.contains("famicom"))
            })
            .map(|choice| choice.value.clone())
            .expect("famicom set listed for player1");
        let index = row(&payload, p1_key)
            .choices
            .iter()
            .position(|choice| choice.value == famicom)
            .expect("famicom index resolves");
        let next = set(&mut snapshot, p1_key, index, 0);
        let covered = row(&next, p2_key);
        for choice in &covered.choices {
            if choice.value.is_null() {
                assert!(choice.enabled, "not-assigned stays pickable");
            } else {
                assert!(!choice.enabled, "covered slot refuses controllers");
                assert!(
                    !choice.reason.is_empty(),
                    "disabled choice names the covering slot"
                );
            }
        }
        // Drafting the impossible combo anyway (tampered client, old
        // file) fails the finish probe loudly instead of saving silence.
        let pad = covered
            .choices
            .iter()
            .position(|choice| {
                choice
                    .value
                    .as_str()
                    .is_some_and(|value| value.contains("standard"))
            })
            .expect("standard pad listed for player2");
        let conflicted = set(&mut snapshot, p2_key, pad, 0);
        assert!(
            conflicted
                .issues
                .iter()
                .any(|issue| issue.contains("conflict")),
            "finish probe reports the slot conflict: {:?}",
            conflicted.issues
        );
    }

    #[test]
    fn capture_round_trips_through_hardware_keys() {
        let (mut snapshot, _) = test_payload();
        let mut capture: CaptureSlot = None;
        // Find any binding capture row on the NES controller screen.
        let target_key = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            let sections = build_sections(&vm, &snapshot, &test_keyboards(), None);
            sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|row| row.kind == super::RowKind::Capture)
                .map(|row| (row.key.clone(), row.target.clone()))
                .expect("capture row exists")
        });
        let (key, _) = target_key;
        let started = apply(
            &mut snapshot,
            &mut capture,
            &mut test_keyboards(),
            &key,
            "start_capture",
            0,
            0,
        );
        let pending = started.capture.expect("capture pending");
        assert!(!pending.prompt.is_empty());
        // Unmapped keycodes and releases must not consume: the platform
        // keeps handling them (volume, Back, ...) while capture stays open.
        let (_, consumed_release) = with_test_deps(|| {
            apply_hardware_key(
                29,
                false,
                &mut snapshot,
                &mut capture,
                &test_keyboards(),
                None,
            )
            .expect("hardware key release probes")
        });
        assert!(!consumed_release, "release is not consumed");
        assert!(capture.is_some(), "capture stays pending on release");
        let (_, consumed_unknown) = with_test_deps(|| {
            apply_hardware_key(
                999,
                true,
                &mut snapshot,
                &mut capture,
                &test_keyboards(),
                None,
            )
            .expect("unknown keycode probes")
        });
        assert!(!consumed_unknown, "unmapped keycode is not consumed");
        assert!(capture.is_some(), "capture stays pending on unknown key");
        let (_, consumed) = with_test_deps(|| {
            apply_hardware_key(
                29,
                true,
                &mut snapshot,
                &mut capture,
                &test_keyboards(),
                None,
            )
            .expect("hardware key applies")
        });
        assert!(consumed, "mapped press is consumed");
        assert!(capture.is_none(), "capture consumes after one key");
    }

    #[test]
    fn clear_binding_unbinds_without_keyboard() {
        let (mut snapshot, _) = test_payload();
        let mut capture: CaptureSlot = None;
        let key = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            let sections = build_sections(&vm, &snapshot, &test_keyboards(), None);
            sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|row| row.kind == super::RowKind::Capture)
                .map(|row| row.key.clone())
                .expect("capture row exists")
        });
        let cleared = apply(
            &mut snapshot,
            &mut capture,
            &mut test_keyboards(),
            &key,
            "clear_binding",
            0,
            0,
        );
        let _ = cleared;
        assert!(capture.is_none());
    }

    #[test]
    fn number_rows_advertise_vm_accepted_bounds() {
        let (mut snapshot, _) = test_payload();
        let bounds: Vec<(String, i64, i64)> = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            let sections = build_sections(&vm, &snapshot, &test_keyboards(), None);
            sections
                .iter()
                .flat_map(|section| &section.rows)
                .filter(|row| row.kind == super::RowKind::Number)
                .map(|row| (row.key.clone(), row.min, row.max))
                .collect()
        });
        assert!(!bounds.is_empty());
        for (key, min, max) in bounds {
            assert!(min < max, "{key}: bounds order");
            set(&mut snapshot, &key, 0, min);
            set(&mut snapshot, &key, 0, max);
        }
    }

    #[test]
    fn empty_slots_surface_live_issues() {
        let (mut snapshot, payload) = test_payload();
        for slot in [
            "system.nes.slot.nes.attachment.player1",
            "system.nes.slot.nes.attachment.player2",
        ] {
            let none = row(&payload, slot)
                .choices
                .iter()
                .position(|choice| choice.value.is_null())
                .expect("unassigned choice exists");
            set(&mut snapshot, slot, none, 0);
        }
        let probed = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            draft_issues(&vm)
        });
        assert!(
            probed
                .iter()
                .any(|issue| issue.contains("At least one controller")),
            "all-None slots explain themselves: {probed:?}"
        );
    }

    #[test]
    fn same_edge_hides_status_area_mode() {
        let (mut snapshot, payload) = test_payload();
        // Defaults (BottomLeft vs TopLeft) show the row.
        assert!(row(&payload, "status.area_mode").visible);
        // Move status to the bottom edge: the row hides.
        let status = row(&payload, "status.position");
        let bottom_right = status
            .choices
            .iter()
            .position(|choice| {
                let label = choice.label.to_lowercase();
                label.contains("bottom") && label.contains("right")
            })
            .expect("bottom-right choice exists");
        let next = set(&mut snapshot, "status.position", bottom_right, 0);
        assert!(
            !row(&next, "status.area_mode").visible,
            "same-edge rule hides the row"
        );
    }

    #[test]
    fn settable_rows_carry_descriptions() {
        let (_snapshot, payload) = test_payload();
        // IA principle 3: every interactive row explains itself. Headers
        // carry no supporting text; info readouts stay bare when the label
        // is self-evident (STATUS, version); capture rows show the value
        // line instead (bound key name, or "Tap to assign").
        let bare: Vec<String> = payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter(|row| {
                matches!(
                    row.kind,
                    super::RowKind::Choice
                        | super::RowKind::Toggle
                        | super::RowKind::Number
                        | super::RowKind::Group
                        | super::RowKind::Button
                ) && row.description.is_empty()
            })
            .map(|row| row.key.clone())
            .collect();
        assert!(bare.is_empty(), "rows without description: {bare:?}");
    }

    #[test]
    fn toggle_and_number_commands_round_trip() {
        let (mut snapshot, _) = test_payload();
        let muted = set(&mut snapshot, "audio.muted", 1, 0);
        assert!(snapshot.local.audio.muted);
        assert_eq!(row(&muted, "audio.muted").selected, 1);

        let volume = set(&mut snapshot, "audio.volume", 0, 42);
        assert_eq!(snapshot.local.audio.master_volume_percent, 42);
        assert_eq!(row(&volume, "audio.volume").number, 42);
    }

    #[test]
    fn choice_command_updates_language_and_system_field() {
        let (mut snapshot, payload) = test_payload();
        let language_row = row(&payload, "general.language");
        let japanese = language_row
            .choices
            .iter()
            .position(|choice| choice.value == serde_json::json!("japanese"))
            .expect("japanese choice exists");
        let next = set(&mut snapshot, "general.language", japanese, 0);
        assert_eq!(snapshot.shared.general.language, AppLanguage::Japanese);

        let filter_row = row(&next, "system.nes.video.filter");
        let rgb = filter_row
            .choices
            .iter()
            .position(|choice| choice.value == serde_json::json!("ntsc_rgb"))
            .expect("ntsc_rgb choice exists");
        let _filter = set(&mut snapshot, "system.nes.video.filter", rgb, 0);
        let nes = snapshot
            .shared
            .systems
            .get(NesFactory.system_id().as_ref())
            .map(|settings| settings.downcast_ref::<NesSettings>().unwrap().video.filter)
            .unwrap();
        assert_eq!(nes, NesVideoFilter::NtscRgb);
    }

    #[test]
    fn unknown_keys_and_indices_fail_without_mutation() {
        let (mut snapshot, _) = test_payload();
        let before = snapshot.clone();
        with_test_deps(|| {
            let bad_key = serde_json::to_string(&WireCommand {
                key: "nope.missing".to_string(),
                index: 0,
                number: 0,
                op: "set".to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_key,
                    &mut None,
                    &mut test_keyboards(),
                    &mut None
                )
                .is_err()
            );

            let bad_index = serde_json::to_string(&WireCommand {
                key: "audio.sample_rate".to_string(),
                index: 99,
                number: 0,
                op: "set".to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_index,
                    &mut None,
                    &mut test_keyboards(),
                    &mut None
                )
                .is_err()
            );

            let bad_op = serde_json::to_string(&WireCommand {
                key: "audio.volume".to_string(),
                index: 0,
                number: 42,
                op: "frobnicate".to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_op,
                    &mut None,
                    &mut test_keyboards(),
                    &mut None
                )
                .is_err()
            );

            let bad_port = serde_json::to_string(&WireCommand {
                key: "devices.evil.kbd:builtin".to_string(),
                index: 1,
                number: 0,
                op: "set".to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_port,
                    &mut None,
                    &mut test_keyboards(),
                    &mut None
                )
                .is_err(),
                "unknown ports never persist"
            );

            let bad_scope = serde_json::to_string(&WireCommand {
                key: "reset.evil".to_string(),
                index: 0,
                number: 0,
                op: "reset_section".to_string(),
                payload: serde_json::Value::Null,
            })
            .unwrap();
            assert!(
                apply_command_to(
                    &mut snapshot,
                    &bad_scope,
                    &mut None,
                    &mut test_keyboards(),
                    &mut None
                )
                .is_err(),
                "unknown reset scopes never no-op silently"
            );
        });
        assert_eq!(snapshot, before);
    }

    #[test]
    fn finish_rejects_conflicting_bindings() {
        // `finish` gates on the shared validator: an invalid draft fails.
        let mut snapshot = default_snapshot();
        snapshot.local.audio.master_volume_percent = 100;
        let deps = test_deps();
        assert!(finish_snapshot(&snapshot, &deps.factories, Arc::clone(&deps.rates)).is_ok());
        snapshot.local.audio.latency_ms = 5;
        assert!(finish_snapshot(&snapshot, &deps.factories, Arc::clone(&deps.rates)).is_err());
    }
}
