pub mod descriptor;
pub mod load;
pub mod settings;

use std::collections::HashMap;

use nerust_input_traits::{
    AttachmentId, ControlId, DigitalControlId, FieldId, InputAssignments, InputSystemFactory,
    InputValue, UnifiedFieldMap,
};
use thiserror::Error;

use crate::{audio::AudioBackend, identity::SystemId};

use self::{
    descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
    load::{DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest},
    settings::FactorySettingsView,
};

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("core creation failed: {0}")]
    Create(String),
    #[error("invalid settings choice: {0}")]
    InvalidChoice(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
    #[error("invalid settings snapshot")]
    InvalidSettings,
}

/// Session-owned GUI input endpoint, behind a port for ISP. Concrete double-buffering lives in
/// `nerust_emu_thread`; cores and shells program to this narrow surface only.
pub trait GuiInputPort: Send {
    /// Write one absolute field of the staged GUI state.
    fn set(
        &mut self,
        field: usize,
        value: InputValue,
    ) -> Result<(), nerust_input_traits::BufferError>;
    /// Typed write; defaults to [`Self::set`].
    fn set_field(
        &mut self,
        field: FieldId,
        value: InputValue,
    ) -> Result<(), nerust_input_traits::BufferError> {
        self.set(field.index(), value)
    }
    /// Publish staged state when dirty; clean calls skip the handoff.
    fn publish(&mut self);
    /// Reset staged state to neutral.
    fn clear(&mut self);
}

/// Split a unified `(attachment, ControlId)` field map into the legacy digital-only projection,
/// reporting dropped analog entries. Session input maps still key on `DigitalControlId`; builders log and debug-assert on drops so a future analog device cannot vanish silently.
pub type DigitalFieldMap = HashMap<(AttachmentId, DigitalControlId), usize>;
/// Unified `(attachment, ControlId)` entries dropped by the digital projection.
pub type DroppedAnalogFields = Vec<(AttachmentId, ControlId)>;

/// Lossless lookup over the unified map: the end-to-end (attachment, control)
/// path that never passes through the lossy digital projection.
pub fn resolve_unified_field(
    map: &UnifiedFieldMap,
    attachment: AttachmentId,
    control: impl Into<ControlId>,
) -> Option<usize> {
    map.get(&(attachment, control.into())).copied()
}

#[must_use]
pub fn split_digital_field_map(
    unified: &UnifiedFieldMap,
) -> (DigitalFieldMap, DroppedAnalogFields) {
    let mut digital = HashMap::with_capacity(unified.len());
    let mut dropped = Vec::new();
    for ((attachment, control), field) in unified {
        match control.as_digital() {
            Some(id) => {
                digital.insert((*attachment, id), *field);
            }
            None => dropped.push((*attachment, *control)),
        }
    }
    (digital, dropped)
}

/// Raw parts produced by a system factory before EmuCore wrapping.
pub struct CoreParts {
    pub core: Box<dyn crate::ConsoleCore>,
    /// Session-owned audio backend: the emu thread (not the core) pushes through the rate-control
    /// filter into this. Factories start it; cores never see it.
    pub audio: Box<dyn crate::audio::AudioBackend>,
    pub gui_input: Box<dyn GuiInputPort>,
    /// (attachment, control) → absolute field index
    pub field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    pub render_profile: nerust_render_traits::VideoRenderProfile,
    pub palette: Box<[u32]>,
    pub host_peripherals: crate::peripheral::HostPeripheralHandles,
}

/// Factory encapsulating all per-system knowledge. Frontends talk to systems only through this
/// trait; settings arrive via `FactorySettingsView` (no snapshot dependency).
pub trait CoreFactory: Send + Sync {
    fn system_id(&self) -> Box<dyn SystemId>;

    fn display_name(&self) -> &'static str;

    /// File extensions shown by frontend media pickers.
    /// Media acceptance must still be decided by [`Self::probe_media`].
    fn supported_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn probe_media(&self, media: &MediaObject) -> bool;

    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel;

    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError>;

    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError>;

    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions>;

    fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema>;

    fn create_core_and_adapter(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<CoreParts, FactoryError> {
        self.create_core_and_adapter_with_assignments(
            view,
            speaker,
            &self.input_system_factory().default_assignments(),
        )
    }

    /// Same as create_core_and_adapter but with custom controller assignments.
    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError>;

    /// Presentation-only delta needing no core rebuild. Returns the replacement video assets, or
    /// `None` for the full rebuild path. Default `None` keeps today's rebuild behavior.
    fn presentation_swap_assets(
        &self,
        _before: &FactorySettingsView,
        _after: &FactorySettingsView,
    ) -> Option<(nerust_render_traits::VideoRenderProfile, Box<[u32]>)> {
        None
    }

    /// Returns this factory's input system factory for negotiation.
    fn input_system_factory(&self) -> &dyn InputSystemFactory;

    /// Access the [`SystemDefaults`] facet of this factory. Returns `None` for factories that only
    /// provide core creation (ISP separation — GUI integration methods live on `SystemDefaults`).
    fn as_system_defaults(&self) -> Option<&dyn SystemDefaults> {
        None
    }
}

/// System-specific GUI integration defaults. Separated from [`CoreFactory`] to respect ISP:
/// factories that only provide core creation are not forced to implement label resolution, keyboard binding seeds, or default system settings.
pub trait SystemDefaults: Send + Sync {
    /// Default system-specific settings to seed into the shared settings map.
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        None
    }

    /// Resolve a system-specific label ID to a localized string.
    /// `language` is "ja" or "en". Returns None if unknown (display raw ID).
    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        None
    }

    /// Attachment ID prefix for default keyboard bindings.
    fn default_input_attachment_id(&self) -> Option<&'static str> {
        None
    }

    /// Control ID prefix for default keyboard bindings.
    fn default_input_control_prefix(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_input_traits::AnalogControlId;

    #[test]
    fn unified_lookup_never_passes_the_lossy_projection() {
        use nerust_input_traits::AnalogControlId;
        let digital = DigitalControlId::new("test.a");
        let analog = AnalogControlId::new("test.x");
        let attachment = AttachmentId::new("test.pad");
        let unified = HashMap::from([
            ((attachment, ControlId::digital(digital)), 0),
            ((attachment, ControlId::analog(analog)), 1),
        ]);
        assert_eq!(
            resolve_unified_field(&unified, attachment, digital),
            Some(0)
        );
        assert_eq!(resolve_unified_field(&unified, attachment, analog), Some(1));
        assert_eq!(
            resolve_unified_field(&unified, attachment, DigitalControlId::new("test.z")),
            None
        );
    }

    #[test]
    fn projection_keeps_digital_and_reports_analog() {
        let digital = DigitalControlId::new("test.a");
        let analog = AnalogControlId::new("test.x");
        let attachment = AttachmentId::new("test.pad");
        let unified = HashMap::from([
            ((attachment, ControlId::digital(digital)), 0),
            ((attachment, ControlId::analog(analog)), 1),
        ]);
        let (map, dropped) = split_digital_field_map(&unified);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&(attachment, digital)], 0);
        assert_eq!(dropped, vec![(attachment, ControlId::analog(analog))]);
    }

    #[test]
    fn projection_passes_digital_only_maps_cleanly() {
        let digital = DigitalControlId::new("test.b");
        let attachment = AttachmentId::new("test.pad");
        let unified = HashMap::from([((attachment, ControlId::digital(digital)), 3)]);
        let (map, dropped) = split_digital_field_map(&unified);
        assert!(dropped.is_empty());
        assert_eq!(map[&(attachment, digital)], 3);
    }

    #[test]
    fn projection_handles_empty_and_analog_only_maps() {
        let unified: HashMap<(AttachmentId, ControlId), usize> = HashMap::new();
        let (map, dropped) = split_digital_field_map(&unified);
        assert!(map.is_empty() && dropped.is_empty());
        let analog = ControlId::analog(AnalogControlId::new("test.x"));
        let attachment = AttachmentId::new("test.pad");
        let unified = HashMap::from([((attachment, analog), 5)]);
        let (map, dropped) = split_digital_field_map(&unified);
        assert!(map.is_empty());
        assert_eq!(dropped, vec![(attachment, analog)]);
    }

    #[test]
    fn factory_error_display_names_kind() {
        assert!(
            FactoryError::Create("x".to_string())
                .to_string()
                .contains('x')
        );
        assert!(
            FactoryError::InvalidChoice("y".to_string())
                .to_string()
                .contains('y')
        );
        assert!(!FactoryError::InvalidSettings.to_string().is_empty());
    }
}
