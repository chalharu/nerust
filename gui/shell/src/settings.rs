pub mod bindings;
pub mod defaults;

use nerust_core_traits::audio::{AudioBackend, AudioBackendRegistry, GainBackend};
use nerust_settings_core::model::local::{HostBackendLocalSettings, ScalingMode};

/// Port for building the live audio speaker from local settings.
/// Session code depends on this seam; the registry adapter implements it once.
pub trait AudioSpeakerFactory: Send + Sync {
    /// Build the gain-wrapped speaker for `settings`.
    fn build_speaker(&self, settings: &HostBackendLocalSettings) -> Box<dyn AudioBackend>;
}

impl AudioSpeakerFactory for AudioBackendRegistry {
    fn build_speaker(&self, settings: &HostBackendLocalSettings) -> Box<dyn AudioBackend> {
        build_speaker_impl(self, settings)
    }
}

pub fn build_speaker(
    registry: &AudioBackendRegistry,
    settings: &HostBackendLocalSettings,
) -> Box<dyn AudioBackend> {
    build_speaker_impl(registry, settings)
}

fn build_speaker_impl(
    registry: &AudioBackendRegistry,
    settings: &HostBackendLocalSettings,
) -> Box<dyn AudioBackend> {
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

    let rate = {
        let supported = registry.supported_rates();
        if supported.is_empty() || supported.contains(&sample_rate) {
            sample_rate
        } else {
            supported.last().copied().unwrap_or(48_000)
        }
    };
    let backend = registry.autoselect(rate, u32::from(settings.audio.latency_ms));
    Box::new(GainBackend::new(backend, gain))
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
    use nerust_core_traits::audio::{AudioBackend, NullAudio, StereoSample};
    use nerust_settings_core::model::local::ScalingMode;

    use super::scaling_factor;

    #[test]
    fn null_audio_reports_default_sample_rate() {
        let mut speaker = NullAudio;

        AudioBackend::start(&mut speaker);
        AudioBackend::push(&mut speaker, StereoSample::mono(0.5));
        AudioBackend::pause(&mut speaker);

        assert_eq!(AudioBackend::sample_rate(&speaker), 48_000);
    }

    #[test]
    fn scaling_factor_uses_none_for_fit_mode() {
        assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
        assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
    }
}
