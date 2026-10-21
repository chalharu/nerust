use nerust_core_traits::audio::AudioBackend;
use nerust_gba_core::console_core::GbaConsoleCore;
use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_gba_settings::GbaSettings;
use nerust_input_traits::InputAssignments;
use nerust_render_traits::{
    VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
};
use nerust_system_traits::factory::{CoreParts, FactoryError, settings::FactorySettingsView};

use crate::input_profiles::GBA_ATTACHMENT;

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    mut speaker: Box<dyn AudioBackend>,
    assignments: &InputAssignments,
) -> Result<CoreParts, FactoryError> {
    view.system_config
        .as_deref()
        .and_then(|v| v.downcast_ref::<GbaSettings>())
        .ok_or(FactoryError::InvalidSettings)?;
    if assignments.slots().len() != 1 || assignments.slots()[0].0 != GBA_ATTACHMENT {
        return Err(FactoryError::Create(
            "GBA requires exactly one player1 assignment".to_string(),
        ));
    }
    let profile = assignments.slots()[0]
        .1
        .as_ref()
        .ok_or_else(|| FactoryError::Create("GBA controller is not assigned".to_string()))?;
    if profile.profile_id() != nerust_input_traits::ProfileId::new("gba.standard_pad") {
        return Err(FactoryError::Create(format!(
            "unsupported GBA controller: {}",
            profile.profile_id()
        )));
    }

    let resources = nerust_input_traits::InputResources::new(
        nerust_gba_device::StandardPad::input_fields(&crate::input_profiles::GBA_PORT)
            .into_iter()
            .map(|entry| ((entry.attachment, entry.control), entry.field))
            .collect(),
        Box::new(|| Box::<GbaInputBuffer>::default()),
    );
    // Unified map travels end to end; no lossy projection.
    let (gui_input, emu_input, field_map) =
        nerust_emu_thread::InputSplit::typed_resources::<GbaInputBuffer>(resources)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
    let logical_size = LogicalSize {
        width: 240,
        height: 160,
    };
    // The backend stays silent until `start`; the emu thread owns it after.
    speaker.start();
    let core = GbaConsoleCore::new(emu_input);
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
        host_peripherals: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::audio::NullAudio;
    use nerust_gba_settings::GbaSettings;
    use nerust_input_traits::InputSystemFactory;
    use nerust_system_traits::factory::settings::Language;

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
        let assignments = nerust_input_traits::InputAssignments::new(vec![(GBA_ATTACHMENT, None)]);
        assert!(create_core_and_adapter(&view, Box::new(NullAudio), &assignments).is_err());
    }

    #[test]
    fn starts_speaker_on_build() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        use nerust_core_traits::audio::StereoSample;

        /// Records whether the backend was started: backends hold the
        /// stream silent until `start`, so a missing call means no audio.
        struct StartProbe {
            started: Arc<AtomicBool>,
        }

        impl nerust_core_traits::audio::AudioBackend for StartProbe {
            fn start(&mut self) {
                self.started.store(true, Ordering::SeqCst);
            }

            fn pause(&mut self) {}

            fn push(&mut self, _sample: StereoSample) {}
        }

        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(GbaSettings::default())),
        };
        let assignments = crate::GbaFactory.default_assignments();
        let started = Arc::new(AtomicBool::new(false));
        let speaker = Box::new(StartProbe {
            started: Arc::clone(&started),
        });
        create_core_and_adapter(&view, speaker, &assignments).unwrap();
        assert!(
            started.load(Ordering::SeqCst),
            "factory must start the speaker or playback stays silent"
        );
    }
}
