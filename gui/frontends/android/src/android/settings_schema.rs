//! Android field bindings: one source for descriptors, bounds and keyed decoding.
use std::sync::Arc;

use super::settings_model::AndroidSettings;
use nerust_settings_core::i18n::{UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;
use nerust_settings_core::model::{
    local::{
        NotificationAreaMode, NotificationPosition, RumbleTarget, ScreenOrientation,
        TouchOverlayVisibility,
    },
    shared::StoragePolicy,
};

const VOLUME_MIN: u8 = 0;
const VOLUME_MAX: u8 = 100;
const LATENCY_MIN: u16 = 10;
const LATENCY_MAX: u16 = 200;
const SAMPLE_RATE_CHOICES: &[u32] = &[44_100, 48_000];
const OVERLAY_SCALE_MIN: u8 = 50;
const OVERLAY_SCALE_MAX: u8 = 150;
const OVERLAY_OFFSET_MIN: i8 = -30;
const OVERLAY_OFFSET_MAX: i8 = 30;

/// Ranged-int encoding: Kotlin expands `min..=max` with `unit`.
/// The 101-string tables are never built or shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RangeSpec {
    pub min: i64,
    pub max: i64,
    pub unit: &'static str,
}

/// One settings-dialog row: key, label, choices, and current index travel together.
/// All arrays and codecs derive from `setting_defs`; rows cannot drift from choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingDef {
    pub key: String,
    pub label: String,
    pub choices: Vec<String>,
    pub current: usize,
    /// `Some` for ranged ints: `choices` stays empty, Kotlin expands locally.
    pub range: Option<RangeSpec>,
    /// Same-edge rule resolved in Rust; Kotlin only filters on this flag.
    pub visible: bool,
}

/// Canonical corner order shared by labels and indices.
/// Every position picker follows this sequence; nothing else assumes the mapping.
const CORNERS: [NotificationPosition; 4] = [
    NotificationPosition::BottomLeft,
    NotificationPosition::BottomRight,
    NotificationPosition::TopLeft,
    NotificationPosition::TopRight,
];

fn corner_index(position: NotificationPosition) -> usize {
    CORNERS
        .iter()
        .position(|&corner| corner == position)
        .unwrap_or(0)
}

// Descriptor construction and caching belong to the schema, not the snapshot model.
impl AndroidSettings {
    /// One settings row: key, label, choices, and current index in one struct.
    pub(super) fn setting_defs(&self) -> Vec<SettingDef> {
        defs(self)
    }

    pub(super) fn cached_setting_defs_locked(
        entry: &mut Option<(AndroidSettings, Arc<Vec<SettingDef>>)>,
        current: &AndroidSettings,
    ) -> Arc<Vec<SettingDef>> {
        if let Some((cached, defs)) = entry
            && cached == current
        {
            return Arc::clone(defs);
        }
        let defs = Arc::new(current.setting_defs());
        *entry = Some((current.clone(), Arc::clone(&defs)));
        defs
    }
}

/// Key shared by the visibility rule and the tolerance below: one literal.
const STATUS_AREA_MODE_KEY: &str = "status.area_mode";

struct Binding {
    key: &'static str,
    label: UiText,
    range: Option<RangeSpec>,
    choices: ChoiceSource,
    read: fn(&AndroidSettings) -> usize,
    write: fn(&mut AndroidSettings, usize) -> Option<()>,
    /// Missing keys abort decoding, except rows hidden by the same-edge rule.
    required: bool,
}

/// How a binding's display choices localize. Machine identity (read/write
/// positions) stays language-independent; only these captions ride `language`.
#[derive(Clone, Copy)]
enum ChoiceSource {
    /// Ranged int: no choices, Kotlin expands `min..=max` with `unit`.
    Ranged,
    /// Boolean pair in `[false, true]` order.
    OnOff,
    /// Four corners in [`CORNERS`] order.
    Corners,
    /// Numeric sample rates with a ` Hz` suffix; identical in both languages.
    SampleRates,
    /// Fixed per-variant captions resolved through [`UiText`].
    Fixed(&'static [UiText]),
}

/// Display choices for one binding in the requested language.
fn resolve_choices(source: ChoiceSource, language: AppLanguage) -> Vec<String> {
    match source {
        ChoiceSource::Ranged => Vec::new(),
        ChoiceSource::OnOff => [UiText::Off, UiText::On]
            .iter()
            .map(|key| ui_text(language, *key).to_string())
            .collect(),
        ChoiceSource::Corners => [
            UiText::PosBottomLeft,
            UiText::PosBottomRight,
            UiText::PosTopLeft,
            UiText::PosTopRight,
        ]
        .iter()
        .map(|key| ui_text(language, *key).to_string())
        .collect(),
        ChoiceSource::SampleRates => SAMPLE_RATE_CHOICES
            .iter()
            .map(|rate| format!("{rate} Hz"))
            .collect(),
        ChoiceSource::Fixed(keys) => keys
            .iter()
            .map(|key| ui_text(language, *key).to_string())
            .collect(),
    }
}

/// Static descriptors: literals plus fn pointers, built once.
/// Per-call work stays limited to reading `current` indices.
fn bindings() -> &'static [Binding] {
    static DESCRIPTORS: std::sync::OnceLock<Vec<Binding>> = std::sync::OnceLock::new();
    DESCRIPTORS.get_or_init(build_bindings)
}

fn build_bindings() -> Vec<Binding> {
    macro_rules! choice {
        ($field:ident, $key:literal, $label:ident, [$($value:expr => $text:ident),+ $(,)?]) => {
            Binding {
                key: $key, label: UiText::$label, range: None, required: true,
                choices: ChoiceSource::Fixed(&[$(UiText::$text),+]),
                read: |s| [$($value),+].iter().position(|v| *v == s.$field).unwrap_or_default(),
                write: |s, index| { s.$field = *[$($value),+].get(index)?; Some(()) },
            }
        };
    }
    macro_rules! toggle {
        ($field:ident, $key:literal, $label:ident) => {
            Binding {
                key: $key,
                label: UiText::$label,
                range: None,
                required: true,
                choices: ChoiceSource::OnOff,
                read: |s| usize::from(s.$field),
                write: |s, index| {
                    s.$field = match index {
                        0 => false,
                        1 => true,
                        _ => return None,
                    };
                    Some(())
                },
            }
        };
    }
    macro_rules! range {
        ($field:ident, $key:literal, $label:ident, $min:expr, $max:expr, $unit:literal) => {
            Binding {
                key: $key,
                label: UiText::$label,
                required: true,
                range: Some(RangeSpec {
                    min: i64::from($min),
                    max: i64::from($max),
                    unit: $unit,
                }),
                choices: ChoiceSource::Ranged,
                read: |s| {
                    (i64::from(s.$field).clamp(i64::from($min), i64::from($max)) - i64::from($min))
                        as usize
                },
                write: |s, index| {
                    let value = i64::try_from(index).ok()?.checked_add(i64::from($min))?;
                    if !(i64::from($min)..=i64::from($max)).contains(&value) {
                        return None;
                    }
                    s.$field = value.try_into().ok()?;
                    Some(())
                },
            }
        };
    }
    macro_rules! position {
        ($field:ident, $key:literal, $label:ident) => {
            Binding {
                key: $key,
                label: UiText::$label,
                range: None,
                required: true,
                // Corner order must stay `CORNERS` order; captions resolve
                // through `ChoiceSource::Corners` per language in `defs`.
                choices: ChoiceSource::Corners,
                read: |s| corner_index(s.$field),
                write: |s, index| {
                    s.$field = *CORNERS.get(index)?;
                    Some(())
                },
            }
        };
    }
    let mut bindings = vec![
        toggle!(audio_muted, "audio_muted", Mute),
        range!(
            master_volume_percent,
            "master_volume",
            MasterVolume,
            VOLUME_MIN,
            VOLUME_MAX,
            "%"
        ),
        range!(
            latency_ms,
            "latency_ms",
            AudioLatency,
            LATENCY_MIN,
            LATENCY_MAX,
            " ms"
        ),
        Binding {
            key: "sample_rate",
            label: UiText::SampleRate,
            range: None,
            required: true,
            choices: ChoiceSource::SampleRates,
            read: |s| {
                SAMPLE_RATE_CHOICES
                    .iter()
                    .position(|rate| *rate == s.sample_rate)
                    .unwrap_or(SAMPLE_RATE_CHOICES.len() - 1)
            },
            write: |s, index| {
                s.sample_rate = *SAMPLE_RATE_CHOICES.get(index)?;
                Some(())
            },
        },
        toggle!(vsync, "vsync", Vsync),
        choice!(screen_orientation, "screen.orientation", ScreenOrientation, [ScreenOrientation::Auto => AutoRotate, ScreenOrientation::Portrait => Portrait, ScreenOrientation::Landscape => Landscape]),
        choice!(storage_policy, "storage.policy", SaveStoragePolicy, [StoragePolicy::Sidecar => Sidecar, StoragePolicy::AppSharedData => AppSharedData, StoragePolicy::CustomDirectory => CustomDirectory]),
        choice!(overlay_visibility, "controls.overlay.visibility", TouchOverlay, [TouchOverlayVisibility::Always => Always, TouchOverlayVisibility::Auto => Auto, TouchOverlayVisibility::Hidden => Hidden]),
        range!(
            overlay_opacity_percent,
            "controls.overlay.opacity",
            OverlayOpacity,
            0,
            100,
            "%"
        ),
        range!(
            overlay_scale_percent,
            "controls.overlay.scale",
            OverlayScale,
            OVERLAY_SCALE_MIN,
            OVERLAY_SCALE_MAX,
            "%"
        ),
        range!(
            overlay_vertical_offset_percent,
            "controls.overlay.vertical_offset",
            OverlayVerticalOffset,
            OVERLAY_OFFSET_MIN,
            OVERLAY_OFFSET_MAX,
            "%"
        ),
        toggle!(overlay_haptics, "controls.overlay.haptics", OverlayHaptics),
        toggle!(motion_enabled, "controls.cartridge.motion", CartridgeMotion),
        toggle!(rumble_enabled, "controls.cartridge.rumble", CartridgeRumble),
        range!(
            rumble_strength_percent,
            "controls.cartridge.rumble_strength",
            RumbleStrength,
            0,
            100,
            "%"
        ),
        choice!(rumble_target, "controls.cartridge.rumble_target", RumbleTarget, [RumbleTarget::Auto => Auto, RumbleTarget::Handset => Handset, RumbleTarget::Controller => Controller]),
        toggle!(
            notifications_info_enabled,
            "notifications.info",
            NotificationInfo
        ),
        toggle!(
            notifications_warning_enabled,
            "notifications.warning",
            NotificationWarning
        ),
        toggle!(
            notifications_error_enabled,
            "notifications.error",
            NotificationError
        ),
        position!(
            notifications_position,
            "notifications.position",
            NotificationPosition
        ),
        choice!(notifications_area_mode, "notifications.area_mode", NotificationAreaMode, [NotificationAreaMode::Overlay => AreaOverlay, NotificationAreaMode::Strip => AreaStrip]),
        toggle!(status_enabled, "status.enabled", NotificationStatus),
        position!(
            status_position,
            "status.position",
            NotificationStatusPosition
        ),
        choice!(status_area_mode, "status.area_mode", NotificationStatusAreaMode, [NotificationAreaMode::Overlay => AreaOverlay, NotificationAreaMode::Strip => AreaStrip]),
    ];
    for binding in &mut bindings {
        if binding.key == STATUS_AREA_MODE_KEY {
            binding.required = false;
        }
    }
    bindings
}

/// Draft projection for the dialog: recompute the same-edge rule from corner
/// indices without persisting anything. Out-of-range indices fall back to the
/// first corner, matching the dialog's clamped default selection.
pub(super) fn project_status_area_visible(
    notifications_position_index: usize,
    status_position_index: usize,
) -> bool {
    let top = |index: usize| CORNERS.get(index).copied().unwrap_or(CORNERS[0]).is_top();
    top(notifications_position_index) != top(status_position_index)
}

pub(super) fn defs(current: &AndroidSettings) -> Vec<SettingDef> {
    let language = current.language;
    let mut defs: Vec<_> = bindings()
        .iter()
        .map(|binding| SettingDef {
            key: binding.key.into(),
            label: ui_text(language, binding.label).into(),
            current: (binding.read)(current),
            choices: resolve_choices(binding.choices, language),
            range: binding.range,
            visible: binding.key != STATUS_AREA_MODE_KEY
                || current.notifications_position.is_top() != current.status_position.is_top(),
        })
        .collect();
    defs.extend(current.system_choices.iter().map(|choice| {
        SettingDef {
            key: format!("system.{}.{}", choice.system_id, choice.field_id.as_str()),
            label: choice.label.clone(),
            choices: choice
                .options
                .iter()
                .map(|(_, label)| label.clone())
                .collect(),
            current: choice
                .options
                .iter()
                .position(|(id, _)| id == &choice.selected)
                .unwrap_or_default(),
            range: None,
            visible: true,
        }
    }));
    defs
}

pub(super) fn decode(
    current: &AndroidSettings,
    by_key: &dyn Fn(&str) -> Option<usize>,
) -> Option<AndroidSettings> {
    let mut next = current.clone();
    for binding in bindings() {
        match by_key(binding.key) {
            Some(value) => (binding.write)(&mut next, value)?,
            None if !binding.required => {}
            None => return None,
        }
    }
    for choice in &mut next.system_choices {
        let key = format!("system.{}.{}", choice.system_id, choice.field_id.as_str());
        choice.selected = choice.options.get(by_key(&key)?)?.0.clone();
    }
    Some(next)
}
