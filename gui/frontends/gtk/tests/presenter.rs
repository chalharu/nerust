//! Presenter API smoke tests.
//!
//! These tests verify that the `mapping` module's public API compiles
//! and links correctly from outside the crate. Detailed value tests
//! (roundtrip, fallback, all variants) are in `mapping.rs` unit tests.
//!
//! No GTK display required — runs on all platforms including macOS.

use nerust_gtk::mapping;

/// Verify that all public functions are accessible from outside the crate.
#[test]
fn public_api_is_accessible() {
    // Importing the module and calling each function once confirms the
    // crate's public surface is intact. Actual value correctness is
    // verified in mapping.rs unit tests.
    let _ = mapping::map_language_id;
    let _ = mapping::parse_language_id;
    let _ = mapping::map_storage_policy_id;
    let _ = mapping::parse_storage_policy_id;
    let _ = mapping::map_scaling_id;
    let _ = mapping::parse_scaling_id;
}
