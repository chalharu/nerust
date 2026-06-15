use crate::load::SystemLoadOptions;
use nerust_contract_core::audio::{AudioBackend, AudioBackendKind};
use nerust_gui_settings::local::{AudioSettings, HostBackendLocalSettings};
use nerust_gui_settings::shared::{DesktopSharedSettings, SystemSettings};
use nerust_gui_settings::{
    local::ScalingMode,
    nes::{NesSettings, NesVideoFilter},
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_sound_cpal::CpalAudio;
#[cfg(not(target_os = "android"))]
use nerust_sound_openal::OpenAl;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::CLOCK_RATE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioBackendSpec {
    pub requested_sample_rate: i32,
    pub buffer_width: usize,
    pub buffer_count: usize,
    pub achieved_latency_ms: u32,
    pub gain: f32,
}

pub struct HostedSpeaker {
    inner: HostedSpeakerInner,
}

enum HostedSpeakerInner {
    Cpal(CpalAudio),
    #[cfg(not(target_os = "android"))]
    OpenAl(OpenAl),
    Silent(SilentSpeaker),
}

#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SilentSpeaker {
    sample_rate: u32,
}

#[allow(dead_code)]
impl SilentSpeaker {
    fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
        }
    }
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

pub fn build_speaker(settings: &HostBackendLocalSettings) -> Result<HostedSpeaker, String> {
    let spec = audio_backend_spec(settings.audio.clone());

    let kind = AudioBackendKind::autoselect();
    log::info!("build_speaker: autoselect returned {kind:?}");

    // Tier 1: CPAL (全プラットフォーム)
    if kind != AudioBackendKind::Null {
        match CpalAudio::new(
            u32::try_from(spec.requested_sample_rate).unwrap_or(48_000),
            CLOCK_RATE as u32,
            settings.audio.latency_ms,
        ) {
            Ok(speaker) => {
                log::info!("build_speaker: selected CPAL audio backend (Tier 1)");
                return Ok(HostedSpeaker {
                    inner: HostedSpeakerInner::Cpal(speaker),
                });
            }
            Err(e) => log::warn!("build_speaker: CPAL failed ({e})"),
        }
    }

    // Tier 2: OpenAL (デスクトップのみ)
    #[cfg(not(target_os = "android"))]
    if kind != AudioBackendKind::Null {
        let speaker = OpenAl::with_gain(
            spec.requested_sample_rate,
            CLOCK_RATE as i32,
            spec.buffer_width,
            spec.buffer_count,
            spec.gain,
        );
        log::info!("build_speaker: selected OpenAL audio backend (Tier 2)");
        return Ok(HostedSpeaker {
            inner: HostedSpeakerInner::OpenAl(speaker),
        });
    }

    // Tier 3: Silent (常に利用可能)
    log::info!("build_speaker: no audio device available, using silent speaker (Tier 3)");
    Ok(HostedSpeaker {
        inner: HostedSpeakerInner::Silent(SilentSpeaker::new(
            u32::try_from(spec.requested_sample_rate).unwrap_or(48_000),
        )),
    })
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

impl Sound for HostedSpeaker {
    fn start(&mut self) {
        match &mut self.inner {
            HostedSpeakerInner::Cpal(speaker) => AudioBackend::start(speaker),
            #[cfg(not(target_os = "android"))]
            HostedSpeakerInner::OpenAl(speaker) => AudioBackend::start(speaker),
            HostedSpeakerInner::Silent(speaker) => speaker.start(),
        }
    }

    fn pause(&mut self) {
        match &mut self.inner {
            HostedSpeakerInner::Cpal(speaker) => AudioBackend::pause(speaker),
            #[cfg(not(target_os = "android"))]
            HostedSpeakerInner::OpenAl(speaker) => AudioBackend::pause(speaker),
            HostedSpeakerInner::Silent(speaker) => speaker.pause(),
        }
    }
}

impl MixerInput for HostedSpeaker {
    fn push(&mut self, data: f32) {
        match &mut self.inner {
            HostedSpeakerInner::Cpal(speaker) => AudioBackend::push(speaker, data),
            #[cfg(not(target_os = "android"))]
            HostedSpeakerInner::OpenAl(speaker) => AudioBackend::push(speaker, data),
            HostedSpeakerInner::Silent(speaker) => speaker.push(data),
        }
    }

    fn sample_rate(&self) -> u32 {
        match &self.inner {
            HostedSpeakerInner::Cpal(speaker) => AudioBackend::sample_rate(speaker),
            #[cfg(not(target_os = "android"))]
            HostedSpeakerInner::OpenAl(speaker) => AudioBackend::sample_rate(speaker),
            HostedSpeakerInner::Silent(speaker) => speaker.sample_rate(),
        }
    }
}

impl Sound for SilentSpeaker {
    fn start(&mut self) {}

    fn pause(&mut self) {}
}

impl MixerInput for SilentSpeaker {
    fn push(&mut self, _data: f32) {}

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SilentSpeaker, audio_backend_spec, effective_load_options, filter_type, scaling_factor,
    };
    use crate::load::SystemLoadOptions;
    use crate::settings::defaults::seed::{default_local_settings, default_shared_settings};
    use nerust_contract_core::options::Mmc3IrqVariant;
    use nerust_gui_settings::{
        local::{AudioSettings, ScalingMode},
        nes::NesVideoFilter,
        shared::SystemSettings,
    };
    use nerust_sound_traits::{MixerInput, Sound};

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
    fn silent_speaker_reports_requested_sample_rate() {
        let mut speaker = SilentSpeaker::new(48_000);

        speaker.start();
        speaker.push(0.5);
        speaker.pause();

        assert_eq!(speaker.sample_rate(), 48_000);
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
            nerust_screen_filter::FilterType::NtscSVideo
        ));
    }

    #[test]
    fn scaling_factor_uses_none_for_fit_mode() {
        assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
        assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
    }
}
