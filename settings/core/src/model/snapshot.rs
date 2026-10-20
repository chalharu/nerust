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

/// How the session absorbs a settings delta without (or with) a rebuild.
/// Derived from [`SettingsApplyPlan`] plus session state, so all callers name one strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStrategy {
    /// No runtime state to move: volume-only or nothing.
    Idle,
    /// Swap the audio backend live; the new backend carries volume.
    AudioSwap,
    /// Try a presentation-only swap; fall back to [`Rebuild`](Self::Rebuild) when unavailable.
    PresentationSwap,
    /// Full core reconstruction.
    Rebuild,
}

impl SettingsApplyPlan {
    /// Fold the plan flags into one strategy. `assignments_changed`
    /// forces reconstruction; `has_core` gates the live paths.
    pub fn live_strategy(&self, assignments_changed: bool, has_core: bool) -> LiveStrategy {
        // `session_rebuild_required` folds the rate/latency and visual reasons;
        // the live paths below absorb the audio-only delta.
        let visual_changed = self.session_rebuild_required && !self.audio_backend_changed;
        if self.audio_backend_changed && !visual_changed && !assignments_changed && has_core {
            // Audio-only delta: the swap absorbs the rebuild entirely
            // (the replacement backend carries the new volume too).
            LiveStrategy::AudioSwap
        } else if visual_changed || assignments_changed {
            if visual_changed && !assignments_changed && has_core {
                LiveStrategy::PresentationSwap
            } else {
                LiveStrategy::Rebuild
            }
        } else {
            LiveStrategy::Idle
        }
    }
}

/// Flags describing what changed between two snapshots.
/// Frontends use these flags to decide which subsystems to refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsApplyPlan {
    pub language_changed: bool,
    pub bindings_changed: bool,
    pub persistence_changed: bool,
    pub session_rebuild_required: bool,
    /// Sample-rate/latency (not volume/mute) changed: the session
    /// swaps the audio backend live instead of rebuilding the core.
    pub audio_backend_changed: bool,
    pub audio_volume_changed: bool,
    pub renderer_rebuild_required: bool,
    pub window_settings_changed: bool,
    pub backend_presentation_changed: bool,
    pub scaling_changed: bool,
    pub vsync_changed: bool,
    pub fullscreen_default_changed: bool,
    /// Notification presentation only: frontends refresh OSD/modal state.
    pub notifications_changed: bool,
}
