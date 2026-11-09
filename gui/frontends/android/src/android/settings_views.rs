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
    i18n::{UiText, text as ui_text},
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

/// Full screen payload: schema version plus the sections Kotlin renders.
/// The draft snapshot stays in the bridge vault; Kotlin never sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ViewPayload {
    pub schema_version: u32,
    pub sections: Vec<SectionWire>,
}

/// One settings section: id plus pre-resolved label plus rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SectionWire {
    pub id: String,
    pub label: String,
    pub rows: Vec<RowWire>,
}

/// One settings row. `choices` carries both the Kotlin-visible labels and the
/// Rust-typed values (echoed back by index); `number`/`step`/`unit` drive
/// stepper rows; `text` drives read-only info rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RowWire {
    pub key: String,
    pub label: String,
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
    pub unit: String,
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

/// Row presentation kinds. Toggles render as switches, numbers as steppers,
/// infos as static text, headers as subgroup titles; choices render as radio lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RowKind {
    Choice,
    Toggle,
    Number,
    Info,
    Header,
}

/// One pick-list entry: display label plus the opaque typed value Rust
/// deserializes when the index comes back in a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChoiceWire {
    pub value: serde_json::Value,
    pub label: String,
}

/// One settings edit from Kotlin: row key plus choice index plus stepper value.
/// Only the field matching the row kind is read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireCommand {
    pub key: String,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub number: i64,
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

/// Open payload for Kotlin: schema version plus sections.
pub(super) fn open_payload(snapshot: &SettingsSnapshot) -> Result<String, String> {
    with_vm_deps(|deps| {
        let payload = ViewPayload {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sections: build_sections(&build_vm(snapshot, deps)?, snapshot),
        };
        serde_json::to_string(&payload).map_err(|error| error.to_string())
    })
}

/// Apply one wire command to the draft snapshot in place, returning the
/// refreshed sections. Stateless per call except for the passed draft.
pub(super) fn apply_command_to(
    snapshot: &mut SettingsSnapshot,
    command_json: &str,
) -> Result<String, String> {
    let command: WireCommand =
        serde_json::from_str(command_json).map_err(|error| error.to_string())?;
    with_vm_deps(|deps| {
        let vm = build_vm(snapshot, deps)?;
        let sections = build_sections(&vm, snapshot);
        if is_android_local_key(&command.key) {
            apply_android_local(snapshot, &sections, &command)?;
        } else {
            apply_view_model_command(&vm, deps, &sections, &command)?;
            *snapshot = vm.snapshot();
        }
        let refreshed = build_vm(snapshot, deps)?;
        let payload = ViewPayload {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sections: build_sections(&refreshed, snapshot),
        };
        serde_json::to_string(&payload).map_err(|error| error.to_string())
    })
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

fn choice_row<T>(key: &str, label: String, choices: &[(T, String)], selected: &T) -> RowWire
where
    T: Serialize + PartialEq,
{
    let selected_value = serde_json::to_value(selected).unwrap_or(serde_json::Value::Null);
    let wires: Vec<ChoiceWire> = choices
        .iter()
        .map(|(value, label)| ChoiceWire {
            value: serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            label: label.clone(),
        })
        .collect();
    let selected = wires
        .iter()
        .position(|wire| wire.value == selected_value)
        .unwrap_or_default();
    RowWire {
        key: key.to_string(),
        label,
        kind: RowKind::Choice,
        choices: wires,
        selected,
        number: 0,
        step: 0,
        unit: String::new(),
        text: String::new(),
        visible: true,
    }
}

fn toggle_row(key: &str, label: String, language: AppLanguage, value: bool) -> RowWire {
    RowWire {
        selected: usize::from(value),
        ..choice_row(
            key,
            label,
            &[
                (false, ui_text(language, UiText::Off).to_string()),
                (true, ui_text(language, UiText::On).to_string()),
            ],
            &value,
        )
    }
    .with_kind(RowKind::Toggle)
}

fn number_row(key: &str, label: String, value: i64, step: i64, unit: &str) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        kind: RowKind::Number,
        choices: Vec::new(),
        selected: 0,
        number: value,
        step,
        unit: unit.to_string(),
        text: String::new(),
        visible: true,
    }
}

fn info_row(key: &str, label: String, text: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        kind: RowKind::Info,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        unit: String::new(),
        text,
        visible: true,
    }
}

fn header_row(label: String) -> RowWire {
    RowWire {
        key: String::new(),
        label,
        kind: RowKind::Header,
        choices: Vec::new(),
        selected: 0,
        number: 0,
        step: 0,
        unit: String::new(),
        text: String::new(),
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
        build_controls_section(snapshot, language),
        build_storage_section(vm, language),
    ];
    sections.extend(build_system_sections(vm, language));
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
            &choices,
            &selected,
        )
    }];
    SectionWire {
        id: "general".to_string(),
        label: crate::settings_sections::section_label("general", language).to_string(),
        rows,
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
                language,
                video.vsync,
            ),
            choice_row(
                "video.orientation",
                ui_text(language, UiText::ScreenOrientation).to_string(),
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
                language,
                view.muted,
            ),
            number_row(
                "audio.volume",
                ui_text(language, UiText::MasterVolume).to_string(),
                i64::from(view.volume_percent),
                5,
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
                    &choices,
                    &selected,
                )
            },
            number_row(
                "audio.latency",
                ui_text(language, UiText::AudioLatency).to_string(),
                i64::from(view.latency_ms),
                5,
                " ms",
            ),
        ],
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
                language,
                view.info_enabled,
            ),
            number_row(
                "notifications.info_duration",
                t(UiText::NotificationDuration),
                i64::from(view.info_duration_ms),
                1000,
                " ms",
            ),
            choice_row(
                "notifications.info_color",
                t(UiText::NotificationInfo),
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
                language,
                view.warning_enabled,
            ),
            number_row(
                "notifications.warning_duration",
                t(UiText::NotificationDuration),
                i64::from(view.warning_duration_ms),
                1000,
                " ms",
            ),
            choice_row(
                "notifications.warning_color",
                t(UiText::NotificationWarning),
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
                language,
                view.error_enabled,
            ),
            toggle_row(
                "notifications.error_modal",
                t(UiText::NotificationErrorModal),
                language,
                view.error_modal,
            ),
            choice_row(
                "notifications.error_color",
                t(UiText::NotificationError),
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
                language,
                view.status_enabled,
            ),
            choice_row(
                "status.position",
                t(UiText::NotificationStatusPosition),
                &view
                    .position_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.status_position,
            ),
            choice_row(
                "status.area_mode",
                t(UiText::NotificationStatusAreaMode),
                &view
                    .area_mode_choices
                    .iter()
                    .map(|choice| (choice.value, choice.label.clone()))
                    .collect::<Vec<_>>(),
                &view.status_area_mode,
            ),
        ],
    }
}

fn build_controls_section(snapshot: &SettingsSnapshot, language: AppLanguage) -> SectionWire {
    let overlay = &snapshot.local.touch_overlay;
    let peripherals = &snapshot.local.cartridge_peripherals;
    let t = |text: UiText| ui_text(language, text).to_string();
    SectionWire {
        id: "controls".to_string(),
        label: crate::settings_sections::section_label("controls", language).to_string(),
        rows: vec![
            choice_row(
                "controls.overlay_visibility",
                t(UiText::TouchOverlay),
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
            number_row(
                "controls.overlay_opacity",
                t(UiText::OverlayOpacity),
                i64::from(overlay.opacity_percent),
                5,
                "%",
            ),
            number_row(
                "controls.overlay_scale",
                t(UiText::OverlayScale),
                i64::from(overlay.scale_percent),
                5,
                "%",
            ),
            number_row(
                "controls.overlay_vertical_offset",
                t(UiText::OverlayVerticalOffset),
                i64::from(overlay.vertical_offset_percent),
                5,
                "%",
            ),
            toggle_row(
                "controls.overlay_haptics",
                t(UiText::OverlayHaptics),
                language,
                overlay.haptics,
            ),
            toggle_row(
                "controls.cartridge_motion",
                t(UiText::CartridgeMotion),
                language,
                peripherals.motion_enabled,
            ),
            toggle_row(
                "controls.cartridge_rumble",
                t(UiText::CartridgeRumble),
                language,
                peripherals.rumble_enabled,
            ),
            number_row(
                "controls.cartridge_rumble_strength",
                t(UiText::RumbleStrength),
                i64::from(peripherals.rumble_strength_percent),
                5,
                "%",
            ),
            choice_row(
                "controls.cartridge_rumble_target",
                t(UiText::RumbleTarget),
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
    }
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
            &choices,
            &selected,
        )
    }];
    if general.show_storage_directory {
        rows.push(info_row(
            "general.storage_directory",
            ui_text(language, UiText::SaveStorageDirectory).to_string(),
            general.storage_directory.clone(),
        ));
    }
    SectionWire {
        id: "storage".to_string(),
        label: crate::settings_sections::section_label("storage", language).to_string(),
        rows,
    }
}

/// One section per machine: its settings fields plus its controller slot
/// assignment. Keyboard bindings and shortcuts need a physical keyboard, so
/// Android shows neither; binding conflicts vanish with them.
fn build_system_sections(vm: &SettingsViewModel, language: AppLanguage) -> Vec<SectionWire> {
    let inputs: std::collections::HashMap<
        String,
        nerust_gui_viewmodel::settings::dto::InputTabView,
    > = vm
        .inputs()
        .iter()
        .map(|input| {
            let view = input.view.get();
            (view.system_id.to_string(), view)
        })
        .collect();
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
                        &field
                            .choices
                            .iter()
                            .map(|choice| (choice.value.as_str().to_string(), choice.label.clone()))
                            .collect::<Vec<_>>(),
                        &field.selected.as_str().to_string(),
                    )
                })
                .collect();
            if let Some(input) = inputs.get(&view.system_id.to_string()) {
                rows.push(header_row(
                    ui_text(language, UiText::Controller).to_string(),
                ));
                rows.extend(input.slots.iter().map(|slot| {
                    choice_row(
                        &format!("system.{}.slot.{}", view.system_id, slot.slot_id.as_str()),
                        slot.label.clone(),
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
                    )
                }));
            }
            SectionWire {
                id: format!("system.{}", view.system_id),
                label: view.label.clone(),
                rows,
            }
        })
        .collect()
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
        install_vm_deps(test_deps().factories, test_deps().rates);
        let result = operation();
        *VM_DEPS.lock().unwrap() = None;
        result
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

    fn apply(snapshot: &mut SettingsSnapshot, key: &str, index: usize, number: i64) -> ViewPayload {
        with_test_deps(|| {
            let command_json = serde_json::to_string(&WireCommand {
                key: key.to_string(),
                index,
                number,
            })
            .unwrap();
            let out = apply_command_to(snapshot, &command_json)
                .unwrap_or_else(|error| panic!("command failed: {error}"));
            serde_json::from_str(&out).unwrap()
        })
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
    fn machine_sections_merge_fields_and_slots_without_keyboard_content() {
        let (mut snapshot, payload) = test_payload();
        // No separate input/shortcuts sections: one section per machine.
        assert!(
            payload
                .sections
                .iter()
                .all(|section| !section.id.starts_with("input.") && section.id != "shortcuts"),
            "input/shortcuts sections must not exist on Android"
        );
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
        let header = nes
            .rows
            .iter()
            .position(|row| row.kind == super::RowKind::Header)
            .expect("controller header exists");
        assert!(
            nes.rows[header + 1..]
                .iter()
                .any(|row| row.key.starts_with("system.nes.slot.")),
            "slots follow the header: {nes:?}"
        );
        assert!(
            nes.rows
                .iter()
                .all(|row| !row.key.contains("binding") && !row.key.contains("conflict")),
            "no keyboard binding or conflict rows on Android"
        );
        // Slot assignment round-trips through the merged key.
        let slot_key = "system.nes.slot.nes.attachment.player1";
        let slot_row = row(&payload, slot_key);
        let none = slot_row
            .choices
            .iter()
            .position(|choice| choice.value.is_null())
            .expect("unassigned choice exists");
        let refreshed = apply(&mut snapshot, slot_key, none, 0);
        assert_eq!(row(&refreshed, slot_key).selected, none);
    }

    #[test]
    fn toggle_and_number_commands_round_trip() {
        let (mut snapshot, _) = test_payload();
        let muted = apply(&mut snapshot, "audio.muted", 1, 0);
        assert!(snapshot.local.audio.muted);
        assert_eq!(row(&muted, "audio.muted").selected, 1);

        let volume = apply(&mut snapshot, "audio.volume", 0, 42);
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
        let next = apply(&mut snapshot, "general.language", japanese, 0);
        assert_eq!(snapshot.shared.general.language, AppLanguage::Japanese);

        let filter_row = row(&next, "system.nes.video.filter");
        let rgb = filter_row
            .choices
            .iter()
            .position(|choice| choice.value == serde_json::json!("ntsc_rgb"))
            .expect("ntsc_rgb choice exists");
        let _filter = apply(&mut snapshot, "system.nes.video.filter", rgb, 0);
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
            })
            .unwrap();
            assert!(apply_command_to(&mut snapshot, &bad_key).is_err());

            let bad_index = serde_json::to_string(&WireCommand {
                key: "audio.sample_rate".to_string(),
                index: 99,
                number: 0,
            })
            .unwrap();
            assert!(apply_command_to(&mut snapshot, &bad_index).is_err());
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
