//! Android settings wire transport: DTO structs and shared row helpers.
//!
//! Pure transport. The view-model core lives in `vm`; policy lives in
//! `projection` (sections) and `commands` (edits); capture lives in `capture`.

use nerust_gui_viewmodel::settings::SettingsViewModel;
use nerust_settings_core::{editor::CaptureTarget, i18n::UiText};
use serde::{Deserialize, Serialize};

/// Shared resolved-language instance, re-exported so every builder names
/// one type. Constructed once per projection from the draft choice and
/// the vault system `Locale`, then threaded through every builder instead
/// of a bare locale or language. The VM working copy is normalized with
/// the same value, so labels from both layers agree.
pub(crate) use nerust_settings_core::i18n::I18n;

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
/// same rule as the runtime gate). Member is the stable port-set id
/// (`pad:<descriptor>`); name is the OS display string. Display-only
/// device truth, never persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GamepadInfo {
    pub member: String,
    pub name: String,
}

/// Bare descriptor behind a `pad:` member id (or the raw descriptor).
pub(super) fn bare_member(member: &str) -> &str {
    member.strip_prefix("pad:").unwrap_or(member)
}

/// Global player ports (not per machine): every system's player-N slot
/// fans out from the same port set. Screen ids derive from the port index,
/// so the table holds port ids only. Lives in the leaf `wire` module so
/// screen builders never import each other for navigation vocabulary.
pub(super) const PLAYER_PORTS: [&str; 2] = [
    nerust_settings_core::model::app_state::PORT_PLAYER_1,
    nerust_settings_core::model::app_state::PORT_PLAYER_2,
];

pub(super) fn player_id(index: usize) -> String {
    format!("player.{}", index + 1)
}

pub(super) fn devices_id(port: &str) -> String {
    format!("devices.{port}")
}

pub(super) fn types_id(index: usize) -> String {
    format!("types.{}", index + 1)
}

pub(super) fn player_label(index: usize, i18n: I18n) -> String {
    i18n.text(if index == 0 {
        UiText::PlayerOne
    } else {
        UiText::PlayerTwo
    })
}

/// Stable row-key fragment for one capture target: shortcuts by action,
/// bindings and overrides by attachment/control (overrides prefix the
/// 1-based pad). Shortcuts return above, so both accessors hold here.
pub(super) fn binding_key(target: &CaptureTarget) -> String {
    if let Some(action) = target.action() {
        return format!("shortcut.{action:?}");
    }
    let (attachment, control) = (
        target.attachment().unwrap_or_default(),
        target.control().unwrap_or_default(),
    );
    match target.pad() {
        Some(pad) => format!("pad{}.{attachment}.{control}", pad + 1),
        None => format!("{attachment}.{control}"),
    }
}

/// Stable settings rank for one pad member: position in the vault
/// order mirror (first-seen descriptor order), which the session uses as
/// the settings index. Live-list positions skew after any disconnect
/// cycle, so screens, radios, and capture all resolve through this —
/// never through the live list position.
pub(crate) fn rank_for_member(pad_order: &[String], member: &str) -> Option<usize> {
    let bare = bare_member(member);
    pad_order.iter().position(|known| known == bare)
}

/// Live info for one stable rank: the mirror names the pad, the scan
/// says whether it is connected right now. `None` is a stale rank
/// (disconnected pad whose settings restore on reconnect) or a rank
/// past the scan, never an error.
pub(crate) fn live_for_rank<'a>(
    gamepads: &'a [GamepadInfo],
    pad_order: &[String],
    rank: usize,
) -> Option<&'a GamepadInfo> {
    let bare = pad_order.get(rank)?;
    gamepads
        .iter()
        .find(|info| bare_member(&info.member) == *bare)
}

/// Stable session order for the display list: vault order follows the
/// session first-seen mirror (refreshed on every connect event), so Pad N
/// names session pad N even after a disconnect/reconnect cycle compacts
/// the scan. Linear scan: at most a handful of pads, no map needed.
pub(crate) fn sort_gamepads_by_order(gamepads: &mut [GamepadInfo], pad_order: &[String]) {
    gamepads.sort_by_key(|info| {
        let bare = bare_member(&info.member);
        rank_for_member(pad_order, bare).unwrap_or(usize::MAX)
    });
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
            (false, i18n.text(UiText::Off)),
            (true, i18n.text(UiText::On)),
        ],
        &value,
    );
    row.kind = RowKind::Toggle;
    row
}

/// Slider bounds for [`number_row`]: one tuple instead of six params.
pub(crate) struct NumberParams {
    pub value: i64,
    pub step: i64,
    pub min: i64,
    pub max: i64,
    pub unit: String,
}

pub(super) fn number_row(
    key: &str,
    label: String,
    description: String,
    params: NumberParams,
) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
        number: params.value,
        step: params.step,
        min: params.min,
        max: params.max,
        unit: params.unit,
        ..RowWire::new(RowKind::Number)
    }
}

pub(super) fn info_row(key: &str, label: String, description: String, text: String) -> RowWire {
    RowWire {
        key: key.to_string(),
        label,
        description,
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
        target: target.to_string(),
        ..RowWire::new(RowKind::Group)
    }
}

/// Key-capture row: tap starts capture, Clear unbinds, Cancel aborts.
/// `description` stays empty: the value line already names the binding.
/// Rows hide when closed (no input device / pad disconnected) instead of
/// disabling in place, so every rendered capture row accepts input.
pub(super) fn capture_row(
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
        text,
        target: target.to_string(),
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

/// Wire form of one capture target: optional fields only, the shape
/// infers the kind (action → shortcut, pad → pad binding, else binding).
/// Lives next to [`encode_target`] so the pair cannot skew; `commands`
/// resolves it with the factory registry.
#[derive(serde::Deserialize)]
pub(super) struct WireTarget {
    #[serde(default)]
    pub action: Option<nerust_settings_core::model::input::ShortcutAction>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub attachment: Option<String>,
    #[serde(default)]
    pub control: Option<String>,
    #[serde(default)]
    pub pad: Option<usize>,
}

/// Opaque capture target: newline-free JSON the key must not parse.
/// The owning system rides from the target itself (shortcuts carry none,
/// the encoder drops it), so callers never re-derive it. The command path
/// decodes the row string once per command.
pub(super) fn encode_target(target: &CaptureTarget) -> String {
    if let Some(action) = target.action() {
        return serde_json::json!({"action": action}).to_string();
    }
    // Bindings and overrides share the shape; overrides add the pad.
    let mut value = serde_json::json!({
        "system": target.system_id().map(|system| system.to_string()).unwrap_or_default(),
        "attachment": target.attachment().unwrap_or_default(),
        "control": target.control().unwrap_or_default(),
    });
    if let Some(pad) = target.pad() {
        value["pad"] = pad.into();
    }
    value.to_string()
}
