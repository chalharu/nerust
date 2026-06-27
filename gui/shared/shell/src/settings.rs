pub mod bindings;
pub mod defaults;
pub mod editor;
pub mod i18n;
pub mod ui_policy;

use std::sync::OnceLock;

use nerust_contract_core::audio::{AudioBackend, AudioBackendRegistry, GainBackend};
use nerust_gui_settings::local::{HostBackendLocalSettings, ScalingMode};

fn build_registry() -> AudioBackendRegistry {
    let mut reg = AudioBackendRegistry::new();
    reg.register(0, &nerust_sound_cpal::CPAL);
    #[cfg(not(target_os = "android"))]
    reg.register(1, &nerust_sound_openal::OPENAL);
    reg
}

pub fn audio_registry() -> &'static AudioBackendRegistry {
    static REGISTRY: OnceLock<AudioBackendRegistry> = OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

pub fn build_speaker(settings: &HostBackendLocalSettings) -> Box<dyn AudioBackend> {
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

    let registry = audio_registry();
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
    use nerust_contract_core::audio::{AudioBackend, NullAudio};
    use nerust_gui_settings::local::ScalingMode;

    use super::scaling_factor;

    #[test]
    fn null_audio_reports_default_sample_rate() {
        let mut speaker = NullAudio;

        AudioBackend::start(&mut speaker);
        AudioBackend::push(&mut speaker, 0.5);
        AudioBackend::pause(&mut speaker);

        assert_eq!(AudioBackend::sample_rate(&speaker), 48_000);
    }

    #[test]
    fn scaling_factor_uses_none_for_fit_mode() {
        assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
        assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
    }
}
