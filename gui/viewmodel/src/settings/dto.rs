use std::fmt;

use nerust_core_traits::identity::SystemId;
use nerust_input_traits::AttachmentId;
use nerust_settings_core::editor::CaptureTarget;
use nerust_settings_core::model::{
    language::AppLanguage,
    local::{NotificationAreaMode, NotificationColor, NotificationPosition, ScalingMode},
    shared::StoragePolicy,
};
use nerust_system_traits::factory::descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId};

/// A labeled choice for pick-list widgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceView<T: Clone + PartialEq + Eq> {
    pub value: T,
    pub label: String,
}

impl<T: Clone + Eq> fmt::Display for ChoiceView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
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

/// Owned lookup of the selected choice. Same predicate as
/// `choice_position`; tao pick-lists and gtk combos share it so
/// profile-id matching cannot drift per toolkit.
pub fn find_choice<T: Clone + PartialEq + Eq>(
    choices: &[ChoiceView<T>],
    value: &T,
) -> Option<ChoiceView<T>> {
    choices.iter().find(|c| &c.value == value).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_view_display_shows_label() {
        let c = ChoiceView {
            value: 42usize,
            label: "Answer".into(),
        };
        assert_eq!(c.to_string(), "Answer");
    }

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
            find_choice(&choices, &2).map(|c| c.label),
            Some("Two".into())
        );
        assert_eq!(find_choice(&choices, &9), None);
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerSlotView {
    pub slot_id: AttachmentId,
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
    pub value: BindingValueView,
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

// ── Capture ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStateView {
    pub target: Option<CaptureTarget>,
    pub prompt: String,
}
