use crate::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

/// Complete settings state for the current host.
///
/// This is a pure data type. Serialization/deserialization, persistence,
/// and apply-plan derivation are handled in `nerust_gui_runtime::settings`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SettingsSnapshot {
    pub shared: DesktopSharedSettings,
    pub local: HostBackendLocalSettings,
    pub app_state: DesktopAppState,
}

/// Flags describing what changed between two snapshots.
///
/// Returned by `derive_apply_plan()` in `nerust_gui_runtime::settings::apply`.
/// Frontends use these flags to decide which subsystems to refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsApplyPlan {
    pub language_changed: bool,
    pub bindings_changed: bool,
    pub persistence_changed: bool,
    pub session_rebuild_required: bool,
    pub audio_volume_changed: bool,
    pub renderer_rebuild_required: bool,
    pub window_settings_changed: bool,
    pub backend_presentation_changed: bool,
    pub scaling_changed: bool,
    pub vsync_changed: bool,
    pub fullscreen_default_changed: bool,
}
