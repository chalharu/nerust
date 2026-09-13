use nerust_core_traits::{
    audio::AudioBackend,
    factory::{CoreParts, FactoryError, settings::FactorySettingsView},
};
use nerust_gba_core::console_core::GbaConsoleCore;
use nerust_gba_settings::GbaSettings;
use nerust_input_traits::{
    ControllerCollection, EmuInput, GuiInput, InputAssignments, InputSystemFactory,
};
use nerust_render_traits::{
    VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};

use crate::input_profiles::GBA_ATTACHMENT;

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    assignments: &InputAssignments,
) -> Result<CoreParts, FactoryError> {
    view.system_config
        .as_deref()
        .and_then(|v| v.downcast_ref::<GbaSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    if assignments.slots.len() != 1 || assignments.slots[0].0 != GBA_ATTACHMENT {
        return Err(FactoryError::Create(
            "GBA requires exactly one player1 assignment".to_string(),
        ));
    }
    let profile = assignments.slots[0]
        .1
        .as_ref()
        .ok_or_else(|| FactoryError::Create("GBA controller is not assigned".to_string()))?;
    if profile.profile_id() != nerust_input_traits::ProfileId::new("gba.standard_pad") {
        return Err(FactoryError::Create(format!(
            "unsupported GBA controller: {}",
            profile.profile_id()
        )));
    }

    let controllers =
        ControllerCollection::new(vec![Box::new(nerust_gba_device::StandardPad::new())]);
    let resources =
        <crate::GbaFactory as InputSystemFactory>::create_split(&crate::GbaFactory, &controllers)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
    let gui_input = GuiInput::from_split(&resources.split);
    let emu_input = EmuInput::from_split(&resources.split);
    create_core_and_adapter_with_inputs(
        view,
        speaker,
        gui_input,
        emu_input,
        resources.field_map,
        controllers,
    )
}

pub(crate) fn create_core_and_adapter_with_inputs(
    _view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    gui_input: GuiInput,
    emu_input: EmuInput,
    field_map: std::collections::HashMap<
        (
            nerust_input_traits::AttachmentId,
            nerust_input_traits::DigitalControlId,
        ),
        usize,
    >,
    _controller_collection: ControllerCollection,
) -> Result<CoreParts, FactoryError> {
    let logical_size = LogicalSize {
        width: 240,
        height: 160,
    };
    let core = GbaConsoleCore::new(speaker, emu_input);
    Ok(CoreParts {
        core: Box::new(core),
        gui_input,
        field_map,
        render_profile: VideoRenderProfile {
            source_logical_size: logical_size,
            logical_size,
            physical_size: PhysicalSize::from(logical_size),
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        },
        palette: Vec::new().into_boxed_slice(),
        host_peripherals: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::audio::NullAudio;
    use nerust_core_traits::factory::settings::Language;
    use nerust_gba_settings::GbaSettings;
    use nerust_input_traits::InputSystemFactory;

    use super::*;

    #[test]
    fn builds_rgba_core_parts() {
        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbaSettings::default())),
        };
        let assignments = crate::GbaFactory.default_assignments();
        let parts = create_core_and_adapter(&view, Box::new(NullAudio), &assignments).unwrap();
        assert_eq!(parts.render_profile.source_logical_size.width, 240);
        assert_eq!(parts.render_profile.source_logical_size.height, 160);
        assert_eq!(parts.render_profile.frame_format, VideoFrameFormat::Rgba);
        assert_eq!(parts.field_map.len(), 10);
    }

    #[test]
    fn rejects_missing_controller() {
        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbaSettings::default())),
        };
        let assignments = nerust_input_traits::InputAssignments {
            slots: vec![(GBA_ATTACHMENT, None)],
        };
        assert!(create_core_and_adapter(&view, Box::new(NullAudio), &assignments).is_err());
    }
}
