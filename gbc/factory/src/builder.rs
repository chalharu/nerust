use nerust_core_traits::{
    audio::AudioBackend,
    factory::{CoreParts, FactoryError, settings::FactorySettingsView},
};
use nerust_gbc_core::console_core::GbcConsoleCore;
use nerust_gbc_settings::GbcSettings;
use nerust_input_traits::{
    ControllerCollection, EmuInput, GuiInput, InputAssignments, InputSystemFactory, ProfileId,
};
use nerust_render_traits::{
    VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};

use crate::input_profiles::GBC_ATTACHMENT;

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    mut speaker: Box<dyn AudioBackend>,
    assignments: &InputAssignments,
) -> Result<CoreParts, FactoryError> {
    view.system_config
        .as_deref()
        .and_then(|value| value.downcast_ref::<GbcSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    if assignments.slots.len() != 1 || assignments.slots[0].0 != GBC_ATTACHMENT {
        return Err(FactoryError::Create(
            "GBC requires exactly one player1 assignment".to_string(),
        ));
    }
    let profile = assignments.slots[0]
        .1
        .as_ref()
        .ok_or_else(|| FactoryError::Create("GBC controller is not assigned".to_string()))?;
    if profile.profile_id() != ProfileId::new("gbc.standard_pad") {
        return Err(FactoryError::Create(format!(
            "unsupported GBC controller: {}",
            profile.profile_id()
        )));
    }

    let controllers =
        ControllerCollection::new(vec![Box::new(nerust_gbc_device::StandardPad::new())]);
    let resources = crate::GbcFactory
        .create_split(&controllers)
        .map_err(|error| FactoryError::Create(error.to_string()))?;
    let gui_input = GuiInput::from_split(&resources.split);
    let emu_input = EmuInput::from_split(&resources.split);
    speaker.start();
    let core = GbcConsoleCore::new_empty(speaker, emu_input);
    let logical_size = LogicalSize {
        width: 160,
        height: 144,
    };
    Ok(CoreParts {
        core: Box::new(core),
        gui_input,
        field_map: resources.field_map,
        render_profile: VideoRenderProfile {
            source_logical_size: logical_size,
            logical_size,
            physical_size: PhysicalSize::from(logical_size),
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        },
        palette: Vec::new().into_boxed_slice(),
    })
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::audio::NullAudio;
    use nerust_core_traits::factory::settings::Language;
    use nerust_gbc_settings::GbcSettings;
    use nerust_input_traits::{InputAssignments, InputSystemFactory};

    use super::*;

    #[test]
    fn builds_rgba_core_parts() {
        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbcSettings::default())),
        };
        let assignments = crate::GbcFactory.default_assignments();
        let parts = create_core_and_adapter(&view, Box::new(NullAudio), &assignments).unwrap();
        assert_eq!(parts.render_profile.source_logical_size.width, 160);
        assert_eq!(parts.render_profile.source_logical_size.height, 144);
        assert_eq!(parts.render_profile.frame_format, VideoFrameFormat::Rgba);
        assert!(parts.palette.is_empty());
        assert_eq!(parts.field_map.len(), 8);
    }

    #[test]
    fn rejects_missing_controller() {
        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbcSettings::default())),
        };
        let assignments = InputAssignments {
            slots: vec![(GBC_ATTACHMENT, None)],
        };
        assert!(create_core_and_adapter(&view, Box::new(NullAudio), &assignments).is_err());
    }
}
