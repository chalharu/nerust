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
        UiText, shortcut_row_description, slot_description, system_field_description,
        text as ui_text,
    },
    model::{
        language::AppLanguage,
        local::{
            NotificationAreaMode, NotificationColor, NotificationPosition, RumbleTarget,
            ScreenOrientation, TouchOverlayVisibility,
        },
        shared::StoragePolicy,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SectionWire {
    pub id: String,
    pub label: String,
    pub rows: Vec<RowWire>,
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// One settings row. `choices` carries both the Kotlin-visible labels and the
/// Rust-typed values (echoed back by index); `number`/`step`/`min`/`max`/`unit`
/// drive slider rows; `text` drives read-only info rows; `description` is the
/// M3 supporting text shown under the label (IA principle 3: every non-header
/// row explains itself); `target` carries the opaque capture target for
/// `capture` rows; `Header`/`Group` rows are non-editable navigation aids.
/// Capture rows keep `description` empty by design: the value line already
/// serves as the副文 (bound key name, or "Tap to assign"), and the IA slot
/// rule shows exactly one of description/value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RowWire {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub kind: RowKind,
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
/// rows into another section, captures as key-capture rows; choices render
/// as radio lists.
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

/// One settings edit from Kotlin: row key plus op. `set` reads the choice
/// index or the slider number; capture ops act on the row's opaque target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireCommand {
    pub key: String,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub number: i64,
    #[serde(default = "default_op_set")]
    pub op: String,
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

/// Open payload for Kotlin: schema version, sections, live issues, capture.
pub(super) fn open_payload(snapshot: &SettingsSnapshot) -> Result<String, String> {
    with_vm_deps(|deps| {
        let vm = build_vm(snapshot, deps)?;
        let payload = ViewPayload {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sections: build_sections(&vm, snapshot),
            issues: draft_issues(&vm),
            capture: None,
        };
        serde_json::to_string(&payload).map_err(|error| error.to_string())
    })
}

/// Pending capture slot: target plus the localized prompt captured at start.
pub(super) type CaptureSlot = Option<(CaptureTarget, String)>;

/// Apply one wire command to the draft snapshot in place, returning the
/// refreshed payload. Stateless per call except for the passed draft and
/// capture slot.
pub(super) fn apply_command_to(
    snapshot: &mut SettingsSnapshot,
    command_json: &str,
    capture: &mut CaptureSlot,
) -> Result<String, String> {
    let command: WireCommand =
        serde_json::from_str(command_json).map_err(|error| error.to_string())?;
    with_vm_deps(|deps| {
        if command.op != "set" {
            apply_capture_command(snapshot, deps, &command, capture)?;
        } else {
            let vm = build_vm(snapshot, deps)?;
            let sections = build_sections(&vm, snapshot);
            if is_android_local_key(&command.key) {
                apply_android_local(snapshot, &sections, &command)?;
            } else {
                apply_view_model_command(&vm, deps, &sections, &command)?;
                *snapshot = vm.snapshot();
            }
        }
        refreshed_payload(snapshot, deps, capture)
    })
}

/// Refreshed payload after any draft mutation: sections, live issues from a
/// non-fatal `finish` probe, and the pending capture state.
fn refreshed_payload(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    capture: &CaptureSlot,
) -> Result<String, String> {
    let vm = build_vm(snapshot, deps)?;
    let payload = ViewPayload {
        schema_version: SETTINGS_SCHEMA_VERSION,
        sections: build_sections(&vm, snapshot),
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
            Ok((refreshed_payload(snapshot, deps, capture)?, true))
        } else {
            Ok((refreshed_payload(snapshot, deps, capture)?, false))
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
) -> Result<(), String> {
    if command.op == "cancel_capture" {
        *capture = None;
        return Ok(());
    }
    let vm = build_vm(snapshot, deps)?;
    let sections = build_sections(&vm, snapshot);
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
    RowWire {
        selected: usize::from(value),
        ..choice_row(
            key,
            label,
            description,
            &[
                (false, ui_text(language, UiText::Off).to_string()),
                (true, ui_text(language, UiText::On).to_string()),
            ],
            &value,
        )
    }
    .with_kind(RowKind::Toggle)
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
        visible: true,
    }
}

trait WithKind {
    fn with_kind(self, kind: RowKind) -> Self;
}

impl WithKind for RowWire {
    fn with_kind(mut self, kind: RowKind) -> Self {
        self.kind = kind;
        self
    }
}

fn build_sections(vm: &SettingsViewModel, snapshot: &SettingsSnapshot) -> Vec<SectionWire> {
    let language = snapshot.shared.general.language;
    let mut sections = vec![
        build_general_section(vm, language),
        build_video_section(vm, snapshot, language),
        build_audio_section(vm, language),
        build_notifications_section(vm, language),
        build_storage_section(vm, language),
    ];
    sections.extend(build_controls_sections(vm, snapshot, language));
    sections.extend(build_system_sections(vm, language));
    sections.extend(build_input_sections(vm, language));
    // Canonical order owned by Rust; machine sections keep factory order
    // after the known taxonomy.
    sections.sort_by_key(|section| {
        crate::settings_sections::SETTINGS_SECTION_IDS
            .iter()
            .position(|id| *id == section.id)
            .unwrap_or(usize::MAX)
    });
    sections
}

fn build_general_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.general.view.get();
    let rows = vec![{
        let (choices, selected) = (
            view.language_choices
                .iter()
                .map(|choice| (choice.value, choice.label.clone()))
                .collect::<Vec<_>>(),
            view.language,
        );
        choice_row(
            "general.language",
            ui_text(language, UiText::Language).to_string(),
            ui_text(language, UiText::DescLanguage).to_string(),
            &choices,
            &selected,
        )
    }];
    SectionWire {
        id: "general".to_string(),
        label: crate::settings_sections::section_label("general", language).to_string(),
        rows,
        visible: true,
    }
}

fn build_video_section(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
) -> SectionWire {
    let video = vm.video.view.get();
    let orientation = snapshot.local.screen_orientation;
    SectionWire {
        id: "video".to_string(),
        label: crate::settings_sections::section_label("video", language).to_string(),
        rows: vec![
            toggle_row(
                "video.vsync",
                ui_text(language, UiText::Vsync).to_string(),
                ui_text(language, UiText::DescVsync).to_string(),
                language,
                video.vsync,
            ),
            choice_row(
                "video.orientation",
                ui_text(language, UiText::ScreenOrientation).to_string(),
                ui_text(language, UiText::DescOrientation).to_string(),
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
        ],
        visible: true,
    }
}

fn build_audio_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.audio.view.get();
    SectionWire {
        id: "audio".to_string(),
        label: crate::settings_sections::section_label("audio", language).to_string(),
        rows: vec![
            toggle_row(
                "audio.muted",
                ui_text(language, UiText::Mute).to_string(),
                ui_text(language, UiText::DescMute).to_string(),
                language,
                view.muted,
            ),
            number_row(
                "audio.volume",
                ui_text(language, UiText::MasterVolume).to_string(),
                ui_text(language, UiText::DescVolume).to_string(),
                i64::from(view.volume_percent),
                1,
                0,
                100,
                "%",
            ),
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
                    ui_text(language, UiText::SampleRate).to_string(),
                    ui_text(language, UiText::DescSampleRate).to_string(),
                    &choices,
                    &selected,
                )
            },
            number_row(
                "audio.latency",
                ui_text(language, UiText::AudioLatency).to_string(),
                ui_text(language, UiText::DescLatency).to_string(),
                i64::from(view.latency_ms),
                5,
                10,
                200,
                " ms",
            ),
        ],
        visible: true,
    }
}

fn build_notifications_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let view = vm.notifications.view.get();
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "notifications".to_string(),
        label: crate::settings_sections::section_label("notifications", language).to_string(),
        rows: vec![
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
        ],
        visible: true,
    }
}

/// Controls quick section plus its two hidden detail screens, reachable via
/// group rows. The overlay detail holds the size/position/transparency
/// sliders (Display mode ›); haptics stays in the quick section so there is
/// exactly one place to toggle it. Global shortcuts live in their own hidden
/// screen (Shortcuts ›): port-independent and always active, never mixed
/// into per-port device assignment.
fn build_controls_sections(
    vm: &SettingsViewModel,
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
) -> Vec<SectionWire> {
    let overlay = &snapshot.local.touch_overlay;
    let peripherals = &snapshot.local.cartridge_peripherals;
    let t = |text: UiText| ui_text(language, text).to_string();
    let quick = SectionWire {
        id: "controls".to_string(),
        label: crate::settings_sections::section_label("controls", language).to_string(),
        rows: vec![
            choice_row(
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
            ),
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
            group_row(
                "controls.shortcuts_link",
                t(UiText::Shortcuts),
                t(UiText::DescShortcutsLink),
                "controls.shortcuts",
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
        ],
        visible: true,
    };
    let overlay_detail = SectionWire {
        id: "controls.overlay_detail".to_string(),
        label: t(UiText::TouchOverlay),
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
            .collect(),
        visible: false,
    };
    vec![quick, overlay_detail, shortcuts]
}

fn build_storage_section(vm: &SettingsViewModel, language: AppLanguage) -> SectionWire {
    let general = vm.general.view.get();
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
            ui_text(language, UiText::SaveStoragePolicy).to_string(),
            ui_text(language, UiText::DescStoragePolicy).to_string(),
            &choices,
            &selected,
        )
    }];
    if general.show_storage_directory {
        rows.push(info_row(
            "general.storage_directory",
            ui_text(language, UiText::SaveStorageDirectory).to_string(),
            ui_text(language, UiText::DescStorageDirectory).to_string(),
            general.storage_directory.clone(),
        ));
    }
    SectionWire {
        id: "storage".to_string(),
        label: crate::settings_sections::section_label("storage", language).to_string(),
        rows,
        visible: true,
    }
}

/// One section per machine: its settings fields plus its controller slot
/// assignment. Keyboard bindings and shortcuts need a physical keyboard, so
/// Android shows neither; binding conflicts vanish with them.
/// One section per machine: settings fields plus a navigation row into the
/// machine's controller screen (`input.<system>`). Keyboard bindings live on
/// the controller screen, one drill-down away from the machine settings.
fn build_system_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
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
            SectionWire {
                id: format!("system.{}", view.system_id),
                label: view.label.clone(),
                rows,
                visible: true,
            }
        })
        .collect()
}

/// One controller screen per machine, reachable from the machine section via
/// the controller group row. Hidden from the category list: same level would
/// repeat machine names. Contains slot assignment plus keyboard bindings
/// (capture-capable now that hardware keys route into the session).
fn build_input_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
    vm.inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
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
            let mut rows: Vec<RowWire> = view
                .slots
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
                                    serde_json::to_value(&choice.value)
                                        .unwrap_or(serde_json::Value::Null),
                                    choice.label.clone(),
                                )
                            })
                            .collect::<Vec<_>>(),
                        &serde_json::to_value(&slot.selected_profile_id)
                            .unwrap_or(serde_json::Value::Null),
                    );
                    for (wire, choice) in row.choices.iter_mut().zip(slot.choices.iter()) {
                        let owner = match &choice.value {
                            None => None,
                            Some(profile) => taken
                                .get(profile)
                                .filter(|owner| *owner != slot.slot_id.as_str()),
                        };
                        wire.enabled = owner.is_none();
                        wire.reason = owner
                            .and_then(|owner| slot_labels.get(owner))
                            .cloned()
                            .unwrap_or_default();
                    }
                    row
                })
                .collect();
            for section in &view.sections {
                rows.push(header_row(section.label.clone()));
                rows.extend(section.rows.iter().map(|binding| {
                    capture_row(
                        &format!(
                            "system.{}.binding.{}",
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
            SectionWire {
                id: format!("input.{}", view.system_id),
                label: view.label.clone(),
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

    fn test_payload() -> (SettingsSnapshot, ViewPayload) {
        with_test_deps(|| {
            let snapshot = default_snapshot();
            let json = open_payload(&snapshot).expect("open payload builds");
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
            })
            .unwrap();
            let out = apply_command_to(snapshot, &command_json, capture)
                .unwrap_or_else(|error| panic!("command failed: {error}"));
            serde_json::from_str(&out).unwrap()
        })
    }

    fn set(snapshot: &mut SettingsSnapshot, key: &str, index: usize, number: i64) -> ViewPayload {
        apply(snapshot, &mut None, key, "set", index, number)
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
    fn machine_sections_link_to_hidden_controller_screens() {
        let (_snapshot, payload) = test_payload();
        // Machines appear once with fields plus a controller group row.
        let nes = payload
            .sections
            .iter()
            .find(|section| section.id == "system.nes")
            .expect("nes section exists");
        assert!(
            nes.rows
                .iter()
                .any(|row| row.key == "system.nes.video.filter"),
            "machine fields stay in the machine section"
        );
        let group = nes
            .rows
            .iter()
            .find(|row| row.kind == super::RowKind::Group)
            .expect("controller group row exists");
        assert_eq!(group.target, "input.nes");
        assert!(
            nes.rows
                .iter()
                .all(|row| !row.key.contains("slot.") && !row.key.contains("binding")),
            "slots and bindings drill one level down"
        );
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
        // Global shortcuts live one drill-down under Controls, next to the
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
                "controls links to {id}"
            );
        }
        let shortcuts = payload
            .sections
            .iter()
            .find(|section| section.id == "controls.shortcuts")
            .expect("shortcuts screen exists");
        assert_eq!(
            shortcuts.rows.len(),
            8,
            "all global shortcut actions stay reachable"
        );
        assert!(
            shortcuts
                .rows
                .iter()
                .all(|row| row.kind == super::RowKind::Capture),
            "shortcuts rows are capturable"
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
    fn capture_round_trips_through_hardware_keys() {
        let (mut snapshot, _) = test_payload();
        let mut capture: CaptureSlot = None;
        // Find any binding capture row on the NES controller screen.
        let target_key = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            let sections = build_sections(&vm, &snapshot);
            sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|row| row.kind == super::RowKind::Capture)
                .map(|row| (row.key.clone(), row.target.clone()))
                .expect("capture row exists")
        });
        let (key, _) = target_key;
        let started = apply(&mut snapshot, &mut capture, &key, "start_capture", 0, 0);
        let pending = started.capture.expect("capture pending");
        assert!(!pending.prompt.is_empty());
        // Unmapped keycodes and releases must not consume: the platform
        // keeps handling them (volume, Back, ...) while capture stays open.
        let (_, consumed_release) = with_test_deps(|| {
            apply_hardware_key(29, false, &mut snapshot, &mut capture)
                .expect("hardware key release probes")
        });
        assert!(!consumed_release, "release is not consumed");
        assert!(capture.is_some(), "capture stays pending on release");
        let (_, consumed_unknown) = with_test_deps(|| {
            apply_hardware_key(999, true, &mut snapshot, &mut capture)
                .expect("unknown keycode probes")
        });
        assert!(!consumed_unknown, "unmapped keycode is not consumed");
        assert!(capture.is_some(), "capture stays pending on unknown key");
        let (_, consumed) = with_test_deps(|| {
            apply_hardware_key(29, true, &mut snapshot, &mut capture).expect("hardware key applies")
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
            let sections = build_sections(&vm, &snapshot);
            sections
                .iter()
                .flat_map(|section| &section.rows)
                .find(|row| row.kind == super::RowKind::Capture)
                .map(|row| row.key.clone())
                .expect("capture row exists")
        });
        let cleared = apply(&mut snapshot, &mut capture, &key, "clear_binding", 0, 0);
        let _ = cleared;
        assert!(capture.is_none());
    }

    #[test]
    fn number_rows_advertise_vm_accepted_bounds() {
        let (mut snapshot, _) = test_payload();
        let bounds: Vec<(String, i64, i64)> = with_test_deps(|| {
            let vm = build_vm(&snapshot, &test_deps()).expect("vm builds");
            let sections = build_sections(&vm, &snapshot);
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
    fn non_capture_rows_carry_descriptions() {
        let (_snapshot, payload) = test_payload();
        // IA principle 3: every row explains itself. Headers carry no
        // supporting text and capture rows show the value line instead
        // (bound key name, or "Tap to assign"), so both stay exempt.
        let bare: Vec<String> = payload
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter(|row| {
                !matches!(row.kind, super::RowKind::Header | super::RowKind::Capture)
                    && row.description.is_empty()
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
            })
            .unwrap();
            assert!(apply_command_to(&mut snapshot, &bad_key, &mut None).is_err());

            let bad_index = serde_json::to_string(&WireCommand {
                key: "audio.sample_rate".to_string(),
                index: 99,
                number: 0,
                op: "set".to_string(),
            })
            .unwrap();
            assert!(apply_command_to(&mut snapshot, &bad_index, &mut None).is_err());
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
