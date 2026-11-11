//! Android settings facade: view-model-driven screen wiring.
//! Dialog gates read the bridge flight; the frontend executes plans.

pub(super) use super::bridge::SettingsDialogResult;
pub(super) use super::settings_bridge::{
    apply_app_language, apply_screen_orientation, is_dialog_open, request_show_settings_dialog,
    reset_transient, take_result,
};
pub(super) use super::settings_views::app_language_tag;
pub(super) use super::settings_views::{finish_snapshot, install_vm_deps};
