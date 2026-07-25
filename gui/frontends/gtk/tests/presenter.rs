//! Presenter API integration tests.
//!
//! These tests verify that the mapping functions are publicly accessible
//! and work correctly as a roundtrip. Detailed value testing is done in
//! `mapping.rs` unit tests (roundtrip + fallback for all variants).
//!
//! No GTK display required — runs on all platforms including macOS.

use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gtk::mapping::*;

#[test]
fn map_language_id_roundtrip() {
    for lang in [AppLanguage::Japanese, AppLanguage::English, AppLanguage::SystemDefault] {
        let id = map_language_id(lang);
        let back = parse_language_id(Some(id));
        assert_eq!(back, lang);
    }
}

#[test]
fn map_storage_policy_id_roundtrip() {
    for policy in [StoragePolicy::AppSharedData, StoragePolicy::CustomDirectory, StoragePolicy::Sidecar] {
        let id = map_storage_policy_id(policy);
        let back = parse_storage_policy_id(Some(id));
        assert_eq!(back, policy);
    }
}

#[test]
fn map_scaling_id_roundtrip() {
    for scaling in [ScalingMode::FitToWindow, ScalingMode::X1, ScalingMode::X2, ScalingMode::X3, ScalingMode::X4, ScalingMode::X5] {
        let id = map_scaling_id(scaling);
        let back = parse_scaling_id(Some(id));
        assert_eq!(back, scaling);
    }
}

#[test]
fn fallback_values_are_stable() {
    assert_eq!(parse_language_id(None), AppLanguage::SystemDefault);
    assert_eq!(parse_storage_policy_id(None), StoragePolicy::Sidecar);
    assert_eq!(parse_scaling_id(None), ScalingMode::FitToWindow);
}
