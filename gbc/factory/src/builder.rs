use nerust_core_traits::{
    audio::AudioBackend,
    peripheral::{HostPeripheralHandles, accelerometer_channel, rumble_channel},
};
use nerust_gbc_core::console_core::GbcConsoleCore;
use nerust_gbc_core::input_types::GbcInputBuffer;
use nerust_gbc_settings::GbcSettings;
use nerust_input_traits::{InputAssignments, ProfileId};
use nerust_render_traits::{
    VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};
use nerust_system_traits::factory::{CoreParts, FactoryError, settings::FactorySettingsView};

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
    if assignments.slots().len() != 1 || assignments.slots()[0].0 != GBC_ATTACHMENT {
        return Err(FactoryError::Create(
            "GBC requires exactly one player1 assignment".to_string(),
        ));
    }
    let profile = assignments.slots()[0]
        .1
        .as_ref()
        .ok_or_else(|| FactoryError::Create("GBC controller is not assigned".to_string()))?;
    if profile.profile_id() != ProfileId::new("gbc.standard_pad") {
        return Err(FactoryError::Create(format!(
            "unsupported GBC controller: {}",
            profile.profile_id()
        )));
    }

    let resources = nerust_input_traits::InputResources::new(
        nerust_gbc_device::StandardPad::input_fields(&crate::input_profiles::GBC_PORT)
            .into_iter()
            .map(|entry| ((entry.attachment, entry.control), entry.field))
            .collect(),
        Box::new(|| Box::<GbcInputBuffer>::default()),
    );
    // Unified map travels end to end; no lossy projection.
    let (gui_input, emu_input, field_map) =
        nerust_emu_thread::InputSplit::typed_resources::<GbcInputBuffer>(resources)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
    speaker.start();
    let (accelerometer_handle, accelerometer_port) = accelerometer_channel();
    let (rumble_handle, rumble_port) = rumble_channel();
    let core = GbcConsoleCore::with_peripherals(emu_input, accelerometer_port, rumble_port);
    let logical_size = LogicalSize {
        width: 160,
        height: 144,
    };
    Ok(CoreParts {
        core: Box::new(core),
        audio: speaker,
        gui_input: Box::new(gui_input),
        field_map,
        render_profile: VideoRenderProfile {
            source_logical_size: logical_size,
            logical_size,
            physical_size: PhysicalSize::from(logical_size),
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        },
        palette: Vec::new().into_boxed_slice(),
        host_peripherals: HostPeripheralHandles {
            accelerometer: Some(accelerometer_handle),
            rumble: Some(rumble_handle),
        },
    })
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::audio::NullAudio;
    use nerust_gbc_settings::GbcSettings;
    use nerust_input_traits::{InputAssignments, InputSystemFactory};
    use nerust_system_traits::factory::settings::Language;

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
        assert!(parts.host_peripherals.accelerometer.is_some());
        assert!(parts.host_peripherals.rumble.is_some());
    }

    #[test]
    fn rejects_missing_controller() {
        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbcSettings::default())),
        };
        let assignments = InputAssignments::new(vec![(GBC_ATTACHMENT, None)]);
        assert!(create_core_and_adapter(&view, Box::new(NullAudio), &assignments).is_err());
    }
}
