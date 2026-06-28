use nerust_gui_runtime::settings::SettingsSnapshot;

use crate::session::SessionError;

/// Summary of a settings-apply operation, visible to frontends.
///
/// Replaces the full `SettingsApplyPlan` (which contains implementation
/// details like `session_rebuild_required`, etc.) with only the flags
/// that frontends actually act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsResult {
    /// The emulation core's renderer (GPU surface) needs to be recreated.
    pub renderer_needs_rebuild: bool,
    /// The fullscreen-default setting changed; frontend should sync the
    /// window's fullscreen state.
    pub fullscreen_default_changed: bool,
    /// The window-scaling mode changed; frontend should resize the window.
    pub scaling_changed: bool,
}

/// Frontend-facing session operations.
///
/// Hides `SessionCommand`, `SessionCommandOutcome`, and `SettingsApplyPlan`
/// from frontend code. Each frontend's state struct implements this trait.
pub trait FrontendSession {
    fn pause(&mut self);
    fn resume(&mut self);
    fn toggle_pause(&mut self);
    fn save_active_slot(&mut self);
    fn load_active_slot(&mut self) -> bool;
    fn select_next_slot(&mut self);
    fn select_previous_slot(&mut self);
    fn load_slot(&mut self, slot_id: u64) -> bool;
    fn save_slot(&mut self, slot_id: u64);
    fn delete_slot(&mut self, slot_id: u64);
    fn select_slot(&mut self, slot_id: u64);
    fn create_slot(&mut self);
    fn reset(&mut self);
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError>;
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError>;
}
