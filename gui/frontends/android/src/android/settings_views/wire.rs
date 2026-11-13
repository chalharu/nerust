//! Android settings wire transport: DTO structs, the VM core, and shared helpers.
//!
//! Pure transport plus the process-wide view-model core. Policy lives in
//! `projection` (sections) and `commands` (edits); capture lives in `capture`.

use std::{
    rc::Rc,
    sync::{Arc, OnceLock},
};

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::{SettingsViewModel, StoragePathError, StoragePathValidator};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{
        UiText, language_from_locale_tag, port_count_line, port_summary, slot_description,
        text as ui_text,
    },
    model::language::AppLanguage,
};
use nerust_system_traits::factory::CoreFactory;
use serde::{Deserialize, Serialize};

/// Full screen payload: schema version, sections Kotlin renders, live
/// validation issues for the draft, and the pending key-capture state.
/// The draft snapshot stays in the bridge vault; Kotlin never sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ViewPayload {
    pub schema_version: u32,
    pub sections: Vec<SectionWire>,
    pub issues: Vec<String>,
    pub capture: Option<CaptureWire>,
}

/// Pending key capture: opaque target plus the localized prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CaptureWire {
    pub target: String,
    pub prompt: String,
}

/// One settings section: id plus pre-resolved label plus rows. Invisible
/// sections stay addressable (group navigation) but leave the category list.
/// `sub` is the navigation breadcrumb parent ("Player 1" under "Devices");
/// empty falls back to the top-level title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SectionWire {
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
/// label, present only where it adds information (self-evident labels stay
/// bare: no restatements, no choice listings, no misplaced instructions); `detail_target` shows a trailing Details
/// button navigating to another section (device rows); `target` carries the
/// opaque capture target for `capture` rows; `Header`/`Group` rows are
/// non-editable navigation aids. `Button` rows ask Kotlin for confirmation
/// first, then send the `reset_section` op named by the row key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RowWire {
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
    /// Whether the row accepts input. Capture rows disable this when no
    /// input device is connected; `reason` explains why so Kotlin never
    /// invents copy.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub reason: String,
}

fn default_true() -> bool {
    true
}

impl RowWire {
    fn new(kind: RowKind) -> Self {
        Self {
            key: String::new(),
            label: String::new(),
            description: String::new(),
            kind,
            detail_target: String::new(),
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
            enabled: true,
            reason: String::new(),
        }
    }
}

/// Row presentation kinds. Toggles render as switches, numbers as sliders,
/// infos as static text, headers as subgroup titles, groups as navigation
/// rows into another section, captures as key-capture rows, buttons as
/// confirm-then-run actions (resets); choices render as radio lists.
/// A non-empty `detail_target` adds a trailing Details button (device rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RowKind {
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
pub(crate) struct ChoiceWire {
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
pub(crate) struct KeyboardInfo {
    pub member: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
}

/// One gamepad the Auto overlay gate counts (Kotlin `InputDevice`,
/// same rule as the runtime gate). Names only: Rust displays them in the
/// overlay detail screen so users see why Auto hides, without logcat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GamepadInfo {
    pub name: String,
}

/// One settings edit from Kotlin: row key plus op. `set` reads the choice
/// index or the slider number; capture ops act on the row's opaque target;
/// `device_truth` upserts the vault's enumerated device lists from
/// `payload` (`keyboards` + `capable` + `gamepads`, `lang` optional);
/// `reset_section` restores one screen (or `all`) to defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireOp {
    #[default]
    Set,
    DeviceTruth,
    ResetSection,
    CancelCapture,
    ClearBinding,
    StartCapture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WireCommand {
    pub key: String,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub op: WireOp,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// View-model dependencies shared across JNI calls. `CoreFactory` is
/// `Send + Sync`; the snapshot travels per call, so no editor state is kept.
pub(crate) struct VmDeps {
    pub factories: Vec<Arc<dyn CoreFactory>>,
    pub rates: Arc<[u32]>,
}

static VM_DEPS: OnceLock<VmDeps> = OnceLock::new();

/// Publish the factories and probed sample rates once at startup.
/// Called from the event thread; JNI commands only read. Later installs
/// are ignored: every caller publishes the same registry and rates.
pub(crate) fn install_vm_deps(factories: Vec<Arc<dyn CoreFactory>>, rates: Arc<[u32]>) {
    let _ = VM_DEPS.set(VmDeps { factories, rates });
}

pub(super) fn with_vm_deps<T>(
    operation: impl FnOnce(&VmDeps) -> Result<T, String>,
) -> Result<T, String> {
    let Some(deps) = VM_DEPS.get() else {
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

pub(super) fn build_vm(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    i18n: I18n,
) -> Result<SettingsViewModel, String> {
    // Display copy only: the VM resolves every label through the snapshot
    // language, so SystemDefault is concretized here (Android never sets
    // the LANG env the shared resolver reads). The draft keeps the user's
    // choice; writebacks restore it via `sync_snapshot_from_vm`.
    let mut resolved = snapshot.clone();
    resolved.shared.general.language = i18n.language();
    SettingsViewModel::new(
        resolved,
        deps.factories.clone(),
        Arc::clone(&deps.rates),
        Rc::new(SafStorageValidator),
    )
    .map_err(|error| error.to_string())
}

/// Resolved display language plus text accessors: constructed once per
/// projection from the draft choice and the vault system `Locale`, then
/// threaded through every builder instead of a bare locale or language.
/// The VM working copy is normalized with the same value, so labels from
/// both layers agree.
#[derive(Debug, Clone, Copy)]
pub(crate) struct I18n {
    language: AppLanguage,
}

impl I18n {
    /// Resolve the display language: an explicit choice wins, SystemDefault
    /// follows the vault-pushed `Locale` (`None` until pushed keeps the
    /// choice for the host fallback).
    pub(crate) fn resolve(setting: AppLanguage, system: Option<AppLanguage>) -> Self {
        let language = match (setting, system) {
            (AppLanguage::SystemDefault, Some(system)) => system,
            _ => setting,
        };
        Self { language }
    }

    pub(crate) fn language(self) -> AppLanguage {
        self.language
    }

    pub(super) fn text(self, key: UiText) -> String {
        ui_text(self.language, key).to_string()
    }

    pub(super) fn port_summary(self, names: &[String]) -> String {
        port_summary(self.language, names)
    }

    pub(super) fn port_count_line(self, count: usize) -> String {
        port_count_line(self.language, count)
    }

    pub(super) fn slot_description(self) -> String {
        slot_description(self.language)
    }

    /// Full disabled-choice sentence, formatted Rust-side so word order
    /// follows the locale (Kotlin renders it verbatim).
    pub(super) fn slot_in_use(self, owner: &str) -> String {
        match self.language {
            AppLanguage::Japanese => format!("{owner}使用中"),
            _ => format!("Used in {owner}"),
        }
    }
}

/// Terminal locale tag to the supported set. Delegates to the shared
/// rule so ingestion and the save push cannot drift from Desktop.
/// The tag *source* stays Android-side (vault-pushed effective tag:
/// saved in-app choice wins), which `sys-locale` cannot see.
pub(crate) fn system_language_from_tag(tag: &str) -> AppLanguage {
    language_from_locale_tag(tag)
}

/// Supported set back to a BCP-47 tag for the save push (empty means
/// the terminal locale). Inverse of `system_language_from_tag`.
pub(crate) fn app_language_tag(language: AppLanguage) -> &'static str {
    match language {
        AppLanguage::Japanese => "ja",
        AppLanguage::English => "en",
        AppLanguage::SystemDefault => "",
    }
}

/// Vault device truth for one projection: enumerated keyboards, touch
/// capability, counted gamepads, and the resolved display language. One
/// `Copy` struct instead of four parallel params (keeps every wire fn
/// under the argument cap).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Projection<'a> {
    pub keyboards: &'a [KeyboardInfo],
    pub touch_capable: Option<bool>,
    pub gamepads: &'a [GamepadInfo],
    pub i18n: I18n,
}

/// Single construction site: every command arm and test projects alike.
pub(crate) fn projection<'a>(
    keyboards: &'a [KeyboardInfo],
    touch_capable: Option<bool>,
    gamepads: &'a [GamepadInfo],
    i18n: I18n,
) -> Projection<'a> {
    Projection {
        keyboards,
        touch_capable,
        gamepads,
        i18n,
    }
}

/// Display copies concretize SystemDefault, so a resolved language readback
/// is ambiguous: restore the draft choice when the value matches the
/// resolved locale (untouched), keeping a genuine explicit change.
pub(super) fn pin_draft_language(
    draft: AppLanguage,
    resolved: AppLanguage,
    target: &mut AppLanguage,
) {
    if *target == resolved {
        *target = draft;
    }
}

/// Write the VM working copy back, keeping the draft language choice
/// (the VM works on the concretized display copy).
pub(super) fn sync_snapshot_from_vm(
    snapshot: &mut SettingsSnapshot,
    vm: &SettingsViewModel,
    i18n: I18n,
) {
    let original = snapshot.shared.general.language;
    let updated = vm.snapshot();
    *snapshot = updated;
    pin_draft_language(
        original,
        i18n.language(),
        &mut snapshot.shared.general.language,
    );
}

/// Pending capture slot: target plus the localized prompt captured at start.
pub(crate) type CaptureSlot = Option<(CaptureTarget, String)>;

/// Non-fatal validation probe: the same gate `finish` enforces at save,
/// surfaced live so the UI can explain disabled/unappliable states.
/// Text resolves in the display language; the view model only carries kinds.
pub(super) fn draft_issues(vm: &SettingsViewModel, i18n: I18n) -> Vec<String> {
    match vm.finish() {
        Ok(_) => Vec::new(),
        Err(state) => state
            .issues
            .iter()
            .map(|issue| issue.message(i18n.language()))
            .collect(),
    }
}

pub(super) fn choice<T: serde::de::DeserializeOwned>(
    row: &RowWire,
    index: usize,
) -> Result<T, String> {
    row.choices
        .get(index)
        .ok_or_else(|| format!("{}: choice index {index} out of range", row.key))
        .and_then(|choice| {
            serde_json::from_value(choice.value.clone())
                .map_err(|error| format!("{}: invalid choice value: {error}", row.key))
        })
}

pub(super) fn toggle_value(row: &RowWire, index: usize) -> Result<bool, String> {
    match index {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!("{}: toggle index {index} out of range", row.key)),
    }
}

pub(super) fn range_value<T>(key: &str, number: i64) -> Result<T, String>
where
    T: TryFrom<i64>,
{
    T::try_from(number).map_err(|_| format!("{key}: value {number} out of range"))
}

pub(super) fn find_row<'a>(sections: &'a [SectionWire], key: &str) -> Result<&'a RowWire, String> {
    sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| !row.key.is_empty() && row.key == key)
        .ok_or_else(|| format!("unknown settings key: {key}"))
}

pub(super) fn choice_row<T>(
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
    let mut selected = 0;
    let mut found = false;
    let wires: Vec<ChoiceWire> = choices
        .iter()
        .enumerate()
        .map(|(index, (value, label))| {
            let wire_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
            if wire_value == selected_value {
                selected = index;
                found = true;
            }
            ChoiceWire {
                value: wire_value,
                label: label.clone(),
                enabled: true,
                reason: String::new(),
            }
        })
        .collect();
    debug_assert!(found, "choice value missing from choices for row {key}");
    RowWire {
        key: key.to_string(),
        label,
        description,
        choices: wires,
        selected,
        ..RowWire::new(RowKind::Choice)
    }
}

pub(super) fn toggle_row(
    key: &str,
    label: String,
    description: String,
    i18n: I18n,
    value: bool,
) -> RowWire {
    let mut row = choice_row(
        key,
        label,
        description,
        &[
            (false, i18n.text(UiText::Off).to_string()),
            (true, i18n.text(UiText::On).to_string()),
        ],
        &value,
    );
    row.kind = RowKind::Toggle;
    row
}

#[allow(
    clippy::too_many_arguments,
    reason = "Wire builder: slider bounds read as one tuple"
)]
pub(super) fn number_row(
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
        number: value,
        step,
        min,
        max,
        unit: unit.to_string(),
        ..RowWire::new(RowKind::Number)
    }
}

pub(super) fn info_row(key: &str, label: String, description: String, text: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Info,
        text,
        ..RowWire::new(RowKind::Info)
    }
}

pub(super) fn header_row(label: String) -> RowWire {
    RowWire {
        label,
        ..RowWire::new(RowKind::Header)
    }
}

/// Navigation row into another section by id.
pub(super) fn group_row(key: &str, label: String, description: String, target: &str) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Group,
        target: target.to_string(),
        ..RowWire::new(RowKind::Group)
    }
}

/// Key-capture row: tap starts capture, Clear unbinds, Cancel aborts.
/// `description` stays empty: the value line already names the binding.
/// Disabled rows (no input device connected) keep Clear: unbinding needs no
/// input device. `reason` carries the localized why.
pub(super) fn capture_row(
    key: &str,
    label: String,
    description: String,
    text: String,
    target: &str,
    enabled: bool,
    reason: &str,
) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        kind: RowKind::Capture,
        text,
        target: target.to_string(),
        enabled,
        reason: reason.to_string(),
        ..RowWire::new(RowKind::Capture)
    }
}

/// Confirm-then-run action row (resets). The row key names the scope:
/// `reset.all` or `reset.<section-id>`. Kotlin confirms first, then sends
/// the `reset_section` op; Rust restores that scope from defaults.
pub(super) fn button_row(key: &str, label: String, description: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        ..RowWire::new(RowKind::Button)
    }
}

/// Single capture gate: capture rows enable (and the command path accepts)
/// exactly when at least one input device is enumerated. Lives in the leaf
/// module so projection and capture share it without a module cycle.
pub(super) fn capture_open(keyboards: &[KeyboardInfo], gamepads: &[GamepadInfo]) -> bool {
    !keyboards.is_empty() || !gamepads.is_empty()
}

pub(super) fn target_system(target: &CaptureTarget, deps: &VmDeps) -> String {
    match target {
        CaptureTarget::Binding { system, .. } => system.to_string(),
        CaptureTarget::Shortcut(_) => deps
            .factories
            .first()
            .map(|factory| factory.system_id().to_string())
            .unwrap_or_default(),
    }
}

/// Opaque capture target: newline-free JSON the key must not parse.
/// The `Value` form is the internal boundary (single serialization at the
/// row edge); the command path decodes the row string once per command.
fn encode_target_value(target: &CaptureTarget, system: &str) -> serde_json::Value {
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
        }),
        CaptureTarget::Shortcut(action) => {
            serde_json::json!({"kind": "shortcut", "action": action})
        }
    }
}

pub(super) fn encode_target(target: &CaptureTarget, system: &str) -> String {
    encode_target_value(target, system).to_string()
}
