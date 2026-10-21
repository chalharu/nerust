//! Android field bindings: one source for descriptors, bounds and keyed decoding.
use super::settings_model::*;
use nerust_settings_core::model::{
    local::{NotificationAreaMode, RumbleTarget, ScreenOrientation, TouchOverlayVisibility},
    shared::StoragePolicy,
};

struct Binding {
    key: &'static str,
    label: &'static str,
    range: Option<RangeSpec>,
    choices: Vec<String>,
    read: fn(&AndroidSettings) -> usize,
    write: fn(&mut AndroidSettings, usize) -> Option<()>,
}

fn bindings() -> Vec<Binding> {
    macro_rules! choice {
        ($field:ident, $key:literal, $label:literal, [$($value:expr => $text:literal),+ $(,)?]) => {
            Binding {
                key: $key, label: $label, range: None,
                choices: vec![$($text.into()),+],
                read: |s| [$($value),+].iter().position(|v| *v == s.$field).unwrap_or_default(),
                write: |s, index| { s.$field = *[$($value),+].get(index)?; Some(()) },
            }
        };
    }
    macro_rules! toggle {
        ($field:ident, $key:literal, $label:literal) => {
            choice!($field, $key, $label, [false => "Off", true => "On"])
        };
    }
    macro_rules! range {
        ($field:ident, $key:literal, $label:literal, $min:expr, $max:expr, $unit:literal) => {
            Binding {
                key: $key,
                label: $label,
                range: Some(RangeSpec {
                    min: i64::from($min),
                    max: i64::from($max),
                    unit: $unit,
                }),
                choices: Vec::new(),
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
        ($field:ident, $key:literal, $label:literal) => {
            Binding {
                key: $key,
                label: $label,
                range: None,
                choices: ["Bottom Left", "Bottom Right", "Top Left", "Top Right"]
                    .map(str::to_string)
                    .to_vec(),
                read: |s| corner_index(s.$field),
                write: |s, index| {
                    s.$field = *CORNERS.get(index)?;
                    Some(())
                },
            }
        };
    }
    vec![
        toggle!(audio_muted, "audio_muted", "Mute"),
        range!(
            master_volume_percent,
            "master_volume",
            "Volume",
            VOLUME_MIN,
            VOLUME_MAX,
            "%"
        ),
        range!(
            latency_ms,
            "latency_ms",
            "Audio Latency (ms)",
            LATENCY_MIN,
            LATENCY_MAX,
            " ms"
        ),
        Binding {
            key: "sample_rate",
            label: "Sample Rate (Hz)",
            range: None,
            choices: SAMPLE_RATE_CHOICES
                .iter()
                .map(|rate| format!("{rate} Hz"))
                .collect(),
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
        toggle!(vsync, "vsync", "VSync"),
        choice!(screen_orientation, "screen.orientation", "Screen Orientation", [ScreenOrientation::Auto => "Auto Rotate", ScreenOrientation::Portrait => "Portrait", ScreenOrientation::Landscape => "Landscape"]),
        choice!(storage_policy, "storage.policy", "Save Location", [StoragePolicy::Sidecar => "Next to ROM", StoragePolicy::AppSharedData => "App Storage", StoragePolicy::CustomDirectory => "Custom Directory"]),
        choice!(overlay_visibility, "controls.overlay.visibility", "Touch Overlay", [TouchOverlayVisibility::Always => "Always", TouchOverlayVisibility::Auto => "Auto", TouchOverlayVisibility::Hidden => "Hidden"]),
        range!(
            overlay_opacity_percent,
            "controls.overlay.opacity",
            "Overlay Opacity",
            0,
            100,
            "%"
        ),
        range!(
            overlay_scale_percent,
            "controls.overlay.scale",
            "Overlay Scale",
            OVERLAY_SCALE_MIN,
            OVERLAY_SCALE_MAX,
            "%"
        ),
        range!(
            overlay_vertical_offset_percent,
            "controls.overlay.vertical_offset",
            "Overlay Vertical Position",
            OVERLAY_OFFSET_MIN,
            OVERLAY_OFFSET_MAX,
            "%"
        ),
        toggle!(
            overlay_haptics,
            "controls.overlay.haptics",
            "Overlay Haptics"
        ),
        toggle!(
            motion_enabled,
            "controls.cartridge.motion",
            "Cartridge Motion Sensor"
        ),
        toggle!(
            rumble_enabled,
            "controls.cartridge.rumble",
            "Cartridge Rumble"
        ),
        range!(
            rumble_strength_percent,
            "controls.cartridge.rumble_strength",
            "Rumble Strength",
            0,
            100,
            "%"
        ),
        choice!(rumble_target, "controls.cartridge.rumble_target", "Rumble Target", [RumbleTarget::Auto => "Auto", RumbleTarget::Handset => "Handset", RumbleTarget::Controller => "Controller"]),
        toggle!(
            notifications_info_enabled,
            "notifications.info",
            "Info Notifications"
        ),
        toggle!(
            notifications_warning_enabled,
            "notifications.warning",
            "Warning Notifications"
        ),
        toggle!(
            notifications_error_enabled,
            "notifications.error",
            "Error Notifications"
        ),
        position!(
            notifications_position,
            "notifications.position",
            "Notification Position"
        ),
        choice!(notifications_area_mode, "notifications.area_mode", "Notification Area", [NotificationAreaMode::Overlay => "Overlay", NotificationAreaMode::Strip => "Strip"]),
        toggle!(status_enabled, "status.enabled", "Status Display"),
        position!(status_position, "status.position", "Status Position"),
        choice!(status_area_mode, "status.area_mode", "Status Area", [NotificationAreaMode::Overlay => "Overlay", NotificationAreaMode::Strip => "Strip"]),
    ]
}

pub(super) fn defs(current: &AndroidSettings) -> Vec<SettingDef> {
    let mut defs: Vec<_> = bindings()
        .into_iter()
        .map(|binding| SettingDef {
            key: binding.key.into(),
            label: binding.label.into(),
            current: (binding.read)(current),
            choices: binding.choices,
            range: binding.range,
            visible: binding.key != "status.area_mode"
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
            None if binding.key == "status.area_mode" => {}
            None => return None,
        }
    }
    for choice in &mut next.system_choices {
        let key = format!("system.{}.{}", choice.system_id, choice.field_id.as_str());
        choice.selected = choice.options.get(by_key(&key)?)?.0.clone();
    }
    Some(next)
}
