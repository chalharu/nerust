use nerust_core_traits::identity::SystemId;
use nerust_input_traits::{AttachmentId, StandardButton};
use nerust_settings_core::editor::CaptureTarget;
use nerust_settings_core::model::{
    input::PadBinding,
    language::AppLanguage,
    local::{
        NotificationAreaMode, NotificationColor, NotificationPosition, RumbleTarget, ScalingMode,
    },
    shared::StoragePolicy,
};
use nerust_system_traits::factory::descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId};

/// A labeled choice for pick-list widgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceView<T: Clone + PartialEq + Eq> {
    pub value: T,
    pub label: String,
}

/// Index of `value` in a shared choice list. Viewmodel order is the
/// contract; frontends must not re-derive it per toolkit.
pub fn choice_position<T: Clone + PartialEq + Eq>(
    choices: &[ChoiceView<T>],
    value: &T,
) -> Option<u32> {
    choices
        .iter()
        .position(|choice| &choice.value == value)
        .map(|index| index as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_lookup_shares_index_and_value() {
        let choices = vec![
            ChoiceView {
                value: 1u32,
                label: "One".into(),
            },
            ChoiceView {
                value: 2u32,
                label: "Two".into(),
            },
        ];
        assert_eq!(choice_position(&choices, &2), Some(1));
        assert_eq!(choice_position(&choices, &9), None);
        assert_eq!(
            choice_position(&choices, &2).map(|i| choices[i as usize].label.clone()),
            Some("Two".into())
        );
    }
}

// ── General ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralView {
    pub language: AppLanguage,
    pub language_choices: Vec<ChoiceView<AppLanguage>>,
    pub storage_policy: StoragePolicy,
    pub storage_policy_choices: Vec<ChoiceView<StoragePolicy>>,
    pub storage_directory: String,
    pub show_storage_directory: bool,
}

// ── Video ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoView {
    pub fullscreen_default: bool,
    pub scaling: ScalingMode,
    pub scaling_choices: Vec<ChoiceView<ScalingMode>>,
    pub vsync: bool,
}

// ── Audio ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioView {
    pub muted: bool,
    pub volume_percent: u8,
    pub sample_rate: u32,
    pub sample_rate_choices: Vec<ChoiceView<u32>>,
    pub latency_ms: u16,
}

// ── Notifications ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationView {
    pub info_enabled: bool,
    pub info_duration_ms: u32,
    pub info_color: NotificationColor,
    pub color_choices: Vec<ChoiceView<NotificationColor>>,
    pub warning_enabled: bool,
    pub warning_duration_ms: u32,
    pub warning_color: NotificationColor,
    pub error_enabled: bool,
    pub error_modal: bool,
    pub error_color: NotificationColor,
    pub status_enabled: bool,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    pub position_choices: Vec<ChoiceView<NotificationPosition>>,
    pub area_mode: NotificationAreaMode,
    pub status_area_mode: NotificationAreaMode,
    pub area_mode_choices: Vec<ChoiceView<NotificationAreaMode>>,
}

// ── System tab ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTabSummary {
    pub system_id: Box<dyn SystemId>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTabView {
    pub system_id: Box<dyn SystemId>,
    pub label: String,
    pub fields: Vec<SystemFieldView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFieldView {
    pub id: SystemSettingsFieldId,
    pub label: String,
    pub selected: SystemSettingsChoiceId,
    pub choices: Vec<ChoiceView<SystemSettingsChoiceId>>,
}

// ── Input tab ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTabView {
    pub system_id: Box<dyn SystemId>,
    pub label: String,
    pub slots: Vec<ControllerSlotView>,
    /// Per-machine binding sections (controller attachments).
    pub sections: Vec<BindingSectionView>,
    pub conflicts: Vec<InputConflictView>,
    /// Per-pad button overrides for this machine, indexed by
    /// connection-order pad. The gamepad tab subscribes to the input
    /// views, so pad writes must surface here: without this field the
    /// projected view compares equal across pad edits and the gamepads
    /// page never rebuilds.
    pub pad_bindings: Vec<Vec<PadBinding>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerSlotView {
    pub slot_id: AttachmentId,
    /// Resolved player port (None for non-player attachments). Frontends
    /// never parse slot strings; the viewmodel resolves once here.
    pub port: Option<usize>,
    pub label: String,
    pub selected_profile_id: Option<String>,
    pub choices: Vec<ChoiceView<Option<String>>>,
    pub occupied_by_other_slot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSectionView {
    pub label: String,
    pub rows: Vec<BindingRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRowView {
    pub target: CaptureTarget,
    pub label: String,
    /// Resolved player port for binding targets (`None` for shortcuts and
    /// non-player attachments). Frontends scope per-pad rows through this
    /// instead of parsing attachment strings.
    pub port: Option<usize>,
    pub value: BindingValueView,
    /// Fixed-layout default pad button for this control, if the console
    /// maps it to one. Per-pad mapping screens show it when the pad
    /// carries no override (never "Unbound" for mapped controls).
    pub pad_default: Option<StandardButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingValueView {
    Unbound(String),
    Bound(String),
    Capturing(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputConflictView {
    pub message: String,
}

// ── Gamepads ─────────────────────────────────────────────────────────

/// Per-pad gamepads screen: each connection-order pad carries its own
/// deadzone, D-pad switch, and player port. Values ride the pad rows.
/// Row labels and port options are built by the frontend (the only
/// reader), so the view carries no per-pad strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamepadTabView {
    pub pads: Vec<PadPortView>,
}

/// One connection-order pad: player port, its own stick deadzone, and
/// the physical D-pad switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadPortView {
    pub index: usize,
    pub port: Option<usize>,
    pub deadzone: u8,
    pub dpad_enabled: bool,
}

// ── Rumble & Motion ────────────────────────────────────────────────

/// Rumble & Motion tab: motion sensor enable, rumble switch, strength,
/// and output target. Values persist ahead of the gamepad backend and
/// apply once it connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RumbleMotionView {
    pub motion_enabled: bool,
    pub rumble_enabled: bool,
    pub rumble_strength_percent: u8,
    pub rumble_target: RumbleTarget,
    pub rumble_target_choices: Vec<ChoiceView<RumbleTarget>>,
}

// ── Capture ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStateView {
    pub target: Option<CaptureTarget>,
    pub prompt: String,
}
