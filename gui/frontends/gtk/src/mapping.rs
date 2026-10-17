//! Pure mapping functions between ViewModel DTOs and GTK widget identifiers.
//!
//! These functions contain no GTK widget types and are fully testable
//! without a display server. They are used by PreferencesBinding to
//! populate widgets and handle signal responses.

use nerust_settings_core::model::{
    language::AppLanguage,
    local::{NotificationAreaMode, NotificationColor, NotificationPosition, ScalingMode},
    shared::StoragePolicy,
};

// ── Forward: enum → GTK id ──────────────────────────────────────────────

/// Map an AppLanguage to the GTK combo active_id string.
pub fn map_language_id(lang: AppLanguage) -> &'static str {
    match lang {
        AppLanguage::Japanese => "japanese",
        AppLanguage::English => "english",
        AppLanguage::SystemDefault => "system_default",
    }
}

/// Map a StoragePolicy to the GTK combo active_id string.
pub fn map_storage_policy_id(policy: StoragePolicy) -> &'static str {
    match policy {
        StoragePolicy::AppSharedData => "app_shared_data",
        StoragePolicy::CustomDirectory => "custom_directory",
        StoragePolicy::Sidecar => "sidecar",
    }
}

/// Map a ScalingMode to the GTK combo active_id string.
pub fn map_scaling_id(scaling: ScalingMode) -> &'static str {
    match scaling {
        ScalingMode::FitToWindow => "fit",
        ScalingMode::X1 => "1",
        ScalingMode::X2 => "2",
        ScalingMode::X3 => "3",
        ScalingMode::X4 => "4",
        ScalingMode::X5 => "5",
    }
}

// ── Reverse: GTK id → enum ──────────────────────────────────────────────

/// Parse a GTK combo active_id string back to AppLanguage.
/// Returns `AppLanguage::SystemDefault` for unknown/unset ids.
pub fn parse_language_id(id: Option<&str>) -> AppLanguage {
    match id {
        Some("japanese") => AppLanguage::Japanese,
        Some("english") => AppLanguage::English,
        _ => AppLanguage::SystemDefault,
    }
}

/// Parse a GTK combo active_id string back to StoragePolicy.
/// Returns `StoragePolicy::Sidecar` for unknown/unset ids.
pub fn parse_storage_policy_id(id: Option<&str>) -> StoragePolicy {
    match id {
        Some("app_shared_data") => StoragePolicy::AppSharedData,
        Some("custom_directory") => StoragePolicy::CustomDirectory,
        _ => StoragePolicy::Sidecar,
    }
}

/// Map a NotificationColor to the GTK combo active_id string.
pub fn map_color_id(color: NotificationColor) -> &'static str {
    match color {
        NotificationColor::White => "white",
        NotificationColor::Yellow => "yellow",
        NotificationColor::Red => "red",
        NotificationColor::Green => "green",
        NotificationColor::Cyan => "cyan",
    }
}

/// Map a NotificationPosition to the GTK combo active_id string.
pub fn map_position_id(position: NotificationPosition) -> &'static str {
    match position {
        NotificationPosition::TopLeft => "top_left",
        NotificationPosition::TopRight => "top_right",
        NotificationPosition::BottomLeft => "bottom_left",
        NotificationPosition::BottomRight => "bottom_right",
    }
}

/// Map a NotificationAreaMode to the GTK combo active_id string.
pub fn map_area_id(mode: NotificationAreaMode) -> &'static str {
    match mode {
        NotificationAreaMode::Overlay => "overlay",
        NotificationAreaMode::Strip => "strip",
    }
}

/// Parse a GTK combo active_id string back to ScalingMode.
/// Unknown/unset ids fall back to `FitToWindow` with a warning: the
/// combo only offers known ids, so anything else is corrupt state.
pub fn parse_scaling_id(id: Option<&str>) -> ScalingMode {
    match id {
        Some("1") => ScalingMode::X1,
        Some("2") => ScalingMode::X2,
        Some("3") => ScalingMode::X3,
        Some("4") => ScalingMode::X4,
        Some("5") => ScalingMode::X5,
        unknown => {
            log::warn!("unknown scaling id {unknown:?}, falling back to fit-to-window");
            ScalingMode::FitToWindow
        }
    }
}

/// Parse a GTK combo active_id string back to NotificationColor.
/// Unknown/unset ids fall back to white with a warning.
pub fn parse_color_id(id: Option<&str>) -> NotificationColor {
    match id {
        Some("yellow") => NotificationColor::Yellow,
        Some("red") => NotificationColor::Red,
        Some("green") => NotificationColor::Green,
        Some("cyan") => NotificationColor::Cyan,
        Some("white") => NotificationColor::White,
        unknown => {
            log::warn!("unknown color id {unknown:?}, falling back to white");
            NotificationColor::White
        }
    }
}

/// Parse a GTK combo active_id string back to NotificationPosition.
/// Unknown/unset ids fall back to bottom-left with a warning.
pub fn parse_position_id(id: Option<&str>) -> NotificationPosition {
    match id {
        Some("top_left") => NotificationPosition::TopLeft,
        Some("top_right") => NotificationPosition::TopRight,
        Some("bottom_left") => NotificationPosition::BottomLeft,
        Some("bottom_right") => NotificationPosition::BottomRight,
        unknown => {
            log::warn!("unknown position id {unknown:?}, falling back to bottom-left");
            NotificationPosition::BottomLeft
        }
    }
}

/// Parse a GTK combo active_id string back to NotificationAreaMode.
/// Unknown/unset ids fall back to overlay with a warning.
pub fn parse_area_id(id: Option<&str>) -> NotificationAreaMode {
    match id {
        Some("overlay") => NotificationAreaMode::Overlay,
        Some("strip") => NotificationAreaMode::Strip,
        unknown => {
            log::warn!("unknown area id {unknown:?}, falling back to overlay");
            NotificationAreaMode::Overlay
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_language_id_roundtrip() {
        for lang in [
            AppLanguage::Japanese,
            AppLanguage::English,
            AppLanguage::SystemDefault,
        ] {
            let id = map_language_id(lang);
            let back = parse_language_id(Some(id));
            assert_eq!(back, lang, "roundtrip failed for {lang:?}");
        }
    }

    #[test]
    fn map_color_id_roundtrip() {
        for color in [
            NotificationColor::White,
            NotificationColor::Yellow,
            NotificationColor::Red,
            NotificationColor::Green,
            NotificationColor::Cyan,
        ] {
            assert_eq!(parse_color_id(Some(map_color_id(color))), color);
        }
    }

    #[test]
    fn map_position_id_roundtrip() {
        for position in [
            NotificationPosition::TopLeft,
            NotificationPosition::TopRight,
            NotificationPosition::BottomLeft,
            NotificationPosition::BottomRight,
        ] {
            assert_eq!(parse_position_id(Some(map_position_id(position))), position);
        }
    }

    #[test]
    fn map_area_id_roundtrip() {
        for mode in [NotificationAreaMode::Overlay, NotificationAreaMode::Strip] {
            assert_eq!(parse_area_id(Some(map_area_id(mode))), mode);
        }
    }

    #[test]
    fn map_storage_policy_id_roundtrip() {
        for policy in [
            StoragePolicy::AppSharedData,
            StoragePolicy::CustomDirectory,
            StoragePolicy::Sidecar,
        ] {
            let id = map_storage_policy_id(policy);
            let back = parse_storage_policy_id(Some(id));
            assert_eq!(back, policy, "roundtrip failed for {policy:?}");
        }
    }

    #[test]
    fn map_scaling_id_roundtrip() {
        for scaling in [
            ScalingMode::FitToWindow,
            ScalingMode::X1,
            ScalingMode::X2,
            ScalingMode::X3,
            ScalingMode::X4,
            ScalingMode::X5,
        ] {
            let id = map_scaling_id(scaling);
            let back = parse_scaling_id(Some(id));
            assert_eq!(back, scaling, "roundtrip failed for {scaling:?}");
        }
    }

    #[test]
    fn parse_language_id_unknown_falls_back() {
        assert_eq!(
            parse_language_id(Some("unknown")),
            AppLanguage::SystemDefault
        );
        assert_eq!(parse_language_id(None), AppLanguage::SystemDefault);
    }

    #[test]
    fn parse_storage_policy_id_unknown_falls_back() {
        assert_eq!(
            parse_storage_policy_id(Some("unknown")),
            StoragePolicy::Sidecar
        );
        assert_eq!(parse_storage_policy_id(None), StoragePolicy::Sidecar);
    }

    #[test]
    fn parse_scaling_id_unknown_falls_back() {
        assert_eq!(parse_scaling_id(Some("unknown")), ScalingMode::FitToWindow);
        assert_eq!(parse_scaling_id(None), ScalingMode::FitToWindow);
    }
}
