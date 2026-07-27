//! Pure mapping functions between ViewModel DTOs and GTK widget identifiers.
//!
//! These functions contain no GTK widget types and are fully testable
//! without a display server. They are used by PreferencesBinding to
//! populate widgets and handle signal responses.

use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};

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

/// Parse a GTK combo active_id string back to ScalingMode.
/// Returns `ScalingMode::FitToWindow` for unknown/unset ids.
pub fn parse_scaling_id(id: Option<&str>) -> ScalingMode {
    match id {
        Some("1") => ScalingMode::X1,
        Some("2") => ScalingMode::X2,
        Some("3") => ScalingMode::X3,
        Some("4") => ScalingMode::X4,
        Some("5") => ScalingMode::X5,
        _ => ScalingMode::FitToWindow,
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
