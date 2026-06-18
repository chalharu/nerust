use crate::load::SystemLoadOptions;
use nerust_contract_core::audio::AudioBackendRegistry;
use nerust_gui_settings::local::HostBackendLocalSettings;
use nerust_gui_settings::shared::{DesktopSharedSettings, SystemSettings};
use nerust_gui_settings::{
    local::ScalingMode,
    nes::{NesSettings, NesVideoFilter},
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_video::FilterType;
use nerust_sound_traits::MixerBridge;
use nerust_timer::CLOCK_RATE;

pub fn build_screen_buffer(settings: &DesktopSharedSettings) -> ScreenBuffer {
    ScreenBuffer::new_gpu(
        filter_type(settings),
        nerust_screen_video::LogicalSize {
            width: 256,
            height: 240,
        },
    )
}

pub fn build_speaker(settings: &HostBackendLocalSettings) -> Result<MixerBridge, String> {
    let sample_rate = if settings.audio.sample_rate > 0 {
        settings.audio.sample_rate
    } else {
        48_000
    };
    let gain = if settings.audio.muted {
        0.0
    } else {
        f32::from(settings.audio.master_volume_percent.min(100)) / 100.0
    };

    let mut registry = AudioBackendRegistry::new();
    registry.register(0, "CPAL", nerust_sound_cpal::factory);
    #[cfg(not(target_os = "android"))]
    registry.register(1, "OpenAL", nerust_sound_openal::factory);
    let backend = registry.autoselect(sample_rate, u32::from(settings.audio.latency_ms));
    Ok(MixerBridge::new(backend, CLOCK_RATE as u32, gain))
}

pub fn effective_load_options(
    settings: &DesktopSharedSettings,
    explicit: SystemLoadOptions,
) -> SystemLoadOptions {
    explicit.with_mmc3_irq_variant(system_settings(settings).core.mmc3_irq_variant)
}

pub fn system_settings(settings: &DesktopSharedSettings) -> NesSettings {
    settings
        .systems
        .get(&nerust_input_schema::SystemId::Nes)
        .map(|settings| match settings {
            SystemSettings::Nes(nes) => nes.clone(),
        })
        .unwrap_or_default()
}

pub fn filter_type(settings: &DesktopSharedSettings) -> FilterType {
    match system_settings(settings).video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
    }
}

pub fn scaling_factor(mode: ScalingMode) -> Option<u32> {
    match mode {
        ScalingMode::FitToWindow => None,
        ScalingMode::X1 => Some(1),
        ScalingMode::X2 => Some(2),
        ScalingMode::X3 => Some(3),
        ScalingMode::X4 => Some(4),
        ScalingMode::X5 => Some(5),
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_load_options, filter_type, scaling_factor};
    use crate::load::SystemLoadOptions;
    use crate::settings::defaults::seed::default_shared_settings;
    use nerust_contract_core::audio::{AudioBackend, NullAudio};
    use nerust_contract_core::options::Mmc3IrqVariant;
    use nerust_gui_settings::{local::ScalingMode, nes::NesVideoFilter, shared::SystemSettings};

    #[test]
    fn null_audio_reports_default_sample_rate() {
        let mut speaker = NullAudio;

        AudioBackend::start(&mut speaker);
        AudioBackend::push(&mut speaker, 0.5);
        AudioBackend::pause(&mut speaker);

        assert_eq!(AudioBackend::sample_rate(&speaker), 48_000);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut settings = default_shared_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let resolved = effective_load_options(
            &settings,
            SystemLoadOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            },
        );

        assert_eq!(resolved.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn saved_nes_filter_maps_to_screen_filter_type() {
        let mut settings = default_shared_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        assert!(matches!(
            filter_type(&settings),
            nerust_screen_video::FilterType::NtscSVideo
        ));
    }

    #[test]
    fn scaling_factor_uses_none_for_fit_mode() {
        assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
        assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
    }
}
