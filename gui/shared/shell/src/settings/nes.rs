use crate::load::SystemLoadOptions;
use nerust_contract_settings::local::{AudioSettings, HostBackendLocalSettings};
use nerust_contract_settings::shared::{DesktopSharedSettings, SystemSettings};
use nerust_contract_settings::{
    local::ScalingMode,
    nes::{NesSettings, NesVideoFilter},
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioBackendSpec {
    pub requested_sample_rate: i32,
    pub buffer_width: usize,
    pub buffer_count: usize,
    pub achieved_latency_ms: u32,
    pub gain: f32,
}

pub fn build_screen_buffer(settings: &DesktopSharedSettings) -> ScreenBuffer {
    ScreenBuffer::new(
        filter_type(settings),
        nerust_screen_logical::LogicalSize {
            width: 256,
            height: 240,
        },
    )
}

pub fn build_speaker(settings: &HostBackendLocalSettings) -> OpenAl {
    let spec = audio_backend_spec(settings.audio.clone());
    OpenAl::with_gain(
        spec.requested_sample_rate,
        CLOCK_RATE as i32,
        spec.buffer_width,
        spec.buffer_count,
        spec.gain,
    )
}

pub fn audio_backend_spec(settings: AudioSettings) -> AudioBackendSpec {
    let requested_sample_rate = i32::try_from(settings.sample_rate).unwrap_or(48_000);
    let target_total_frames =
        ((u64::from(settings.sample_rate) * u64::from(settings.latency_ms)).div_ceil(1_000)).max(1);
    let raw_buffer_width = (target_total_frames / 16).max(1);
    let buffer_width = nearest_power_of_two(raw_buffer_width as usize).clamp(64, 1024);
    let buffer_count = usize::try_from(target_total_frames.div_ceil(buffer_width as u64))
        .unwrap_or(32)
        .clamp(4, 32);
    let achieved_latency_ms = u32::try_from(
        (u64::try_from(buffer_width * buffer_count).unwrap_or(0) * 1_000)
            .div_ceil(u64::from(settings.sample_rate.max(1))),
    )
    .unwrap_or(u32::MAX);
    let gain = if settings.muted {
        0.0
    } else {
        f32::from(settings.master_volume_percent.min(100)) / 100.0
    };
    AudioBackendSpec {
        requested_sample_rate,
        buffer_width,
        buffer_count,
        achieved_latency_ms,
        gain,
    }
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
        .and_then(|settings| match settings {
            SystemSettings::Nes(nes) => Some(nes.clone()),
            SystemSettings::Snes(_) => None,
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

fn nearest_power_of_two(value: usize) -> usize {
    if value <= 1 {
        return 1;
    }
    let lower = value.next_power_of_two() >> 1;
    let upper = value.next_power_of_two();
    if value - lower <= upper - value {
        lower.max(1)
    } else {
        upper
    }
}

#[cfg(test)]
mod tests {
    use super::{audio_backend_spec, effective_load_options, filter_type, scaling_factor};
    use crate::load::SystemLoadOptions;
    use crate::settings::defaults::seed::{default_local_settings, default_shared_settings};
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_settings::{
        local::{AudioSettings, ScalingMode},
        nes::NesVideoFilter,
        shared::SystemSettings,
    };

    #[test]
    fn audio_latency_derivation_rounds_to_supported_buffers() {
        let spec = audio_backend_spec(AudioSettings {
            sample_rate: 48_000,
            latency_ms: 50,
            master_volume_percent: 100,
            muted: false,
        });

        assert_eq!(spec.buffer_width, 128);
        assert_eq!(spec.buffer_count, 19);
        assert!(spec.achieved_latency_ms >= 50);
        assert_eq!(spec.gain, 1.0);
    }

    #[test]
    fn muted_audio_forces_zero_gain() {
        let spec = audio_backend_spec(AudioSettings {
            muted: true,
            ..default_local_settings().audio
        });

        assert_eq!(spec.gain, 0.0);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut settings = default_shared_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap()
        else {
            panic!("expected NES settings");
        };
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
            .unwrap()
        else {
            panic!("expected NES settings");
        };
        nes.video.filter = NesVideoFilter::NtscSVideo;

        assert!(matches!(
            filter_type(&settings),
            nerust_screen_filter::FilterType::NtscSVideo
        ));
    }

    #[test]
    fn scaling_factor_uses_none_for_fit_mode() {
        assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
        assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
    }
}
