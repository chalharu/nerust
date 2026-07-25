use std::fmt;

use nerust_core_traits::{
    factory::descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
    identity::SystemId,
};
use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gui_shell::settings::editor::CaptureTarget;
use nerust_input_traits::AttachmentId;

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
