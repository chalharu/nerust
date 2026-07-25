//! Pure mapping functions from ViewModel DTOs to GTK widget state.
//!
//! These functions contain no GTK widget types and are fully testable
//! without a display server. They are used by PreferencesBinding to
//! populate widgets and can be verified on any platform including macOS.

use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_language_id_all_variants() {
        assert_eq!(map_language_id(AppLanguage::Japanese), "japanese");
        assert_eq!(map_language_id(AppLanguage::English), "english");
        assert_eq!(
            map_language_id(AppLanguage::SystemDefault),
            "system_default"
        );
    }

    #[test]
    fn map_storage_policy_id_all_variants() {
        assert_eq!(
            map_storage_policy_id(StoragePolicy::AppSharedData),
            "app_shared_data"
        );
        assert_eq!(
            map_storage_policy_id(StoragePolicy::CustomDirectory),
            "custom_directory"
        );
        assert_eq!(map_storage_policy_id(StoragePolicy::Sidecar), "sidecar");
    }

    #[test]
    fn map_scaling_id_all_variants() {
        assert_eq!(map_scaling_id(ScalingMode::FitToWindow), "fit");
        assert_eq!(map_scaling_id(ScalingMode::X1), "1");
        assert_eq!(map_scaling_id(ScalingMode::X2), "2");
        assert_eq!(map_scaling_id(ScalingMode::X3), "3");
        assert_eq!(map_scaling_id(ScalingMode::X4), "4");
        assert_eq!(map_scaling_id(ScalingMode::X5), "5");
    }
}
