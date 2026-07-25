//! Presenter logic tests — pure mapping functions, no GTK display required.
//!
//! These tests verify that ViewModel state is correctly mapped to GTK
//! widget state without initializing GTK. They run on all platforms
//! including macOS (no display needed).

use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gtk::mapping::*;

#[test]
fn map_language_id_all_variants() {
    assert_eq!(map_language_id(AppLanguage::Japanese), "japanese");
    assert_eq!(map_language_id(AppLanguage::English), "english");
    assert_eq!(map_language_id(AppLanguage::SystemDefault), "system_default");
}

#[test]
fn map_storage_policy_id_all_variants() {
    assert_eq!(map_storage_policy_id(StoragePolicy::AppSharedData), "app_shared_data");
    assert_eq!(map_storage_policy_id(StoragePolicy::CustomDirectory), "custom_directory");
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

#[test]
fn language_mapping_is_bijective() {
    // Every GTK active_id maps back to the correct AppLanguage
    use std::collections::HashMap;
    let mut seen = HashMap::new();
    for (lang, id) in [
        (AppLanguage::Japanese, "japanese"),
        (AppLanguage::English, "english"),
        (AppLanguage::SystemDefault, "system_default"),
    ] {
        assert!(!seen.contains_key(id), "duplicate id: {id}");
        seen.insert(id, lang);
        assert_eq!(map_language_id(lang), id);
    }
}
