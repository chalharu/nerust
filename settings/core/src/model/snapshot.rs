use super::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

/// Complete settings state for the current host.
/// Pure data; persistence and apply plans live in `nerust_gui_runtime::settings`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SettingsSnapshot {
    pub shared: DesktopSharedSettings,
    pub local: HostBackendLocalSettings,
    pub app_state: DesktopAppState,
}
