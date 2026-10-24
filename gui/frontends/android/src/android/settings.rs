//! Android settings facade: model, codec, and bridge re-exported.
//! Dialog gates read the bridge; the frontend executes plans.

pub(super) use super::bridge::SettingsDialogResult;
pub(super) use super::settings_bridge::{
    apply_screen_orientation, cached_setting_defs, cloned_cached_settings, is_dialog_open,
    request_show_settings_dialog, reset_transient, take_result, update_cached_settings,
};
pub(super) use super::settings_codec::{SettingsApplier, SettingsPlan};
pub(super) use super::settings_model::AndroidSettings;
