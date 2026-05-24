use crate::load::NesLoadOptions;
use nerust_contract_settings::{
    desktop::{DesktopSettings, SystemSettings},
    nes::{NesSettings, NesVideoFilter},
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;

pub fn build_screen_buffer(settings: &DesktopSettings) -> ScreenBuffer {
    ScreenBuffer::new_nes_gpu(filter_type(settings))
}

pub fn build_speaker(settings: &DesktopSettings) -> OpenAl {
    let buffer_width = settings.audio.buffer_size.max(32) as usize;
    let requested_sample_rate = settings.audio.sample_rate.max(8_000) as i32;
    let buffer_duration_ms =
        ((buffer_width as u32) * 1_000).div_ceil(settings.audio.sample_rate.max(1));
    let buffer_count = settings
        .audio
        .latency_ms
        .max(buffer_duration_ms)
        .div_ceil(buffer_duration_ms.max(1))
        .max(2) as usize;
    let gain = if settings.audio.muted {
        0.0
    } else {
        settings.audio.master_volume.clamp(0.0, 1.0)
    };
    OpenAl::with_gain(
        requested_sample_rate,
        CLOCK_RATE as i32,
        buffer_width,
        buffer_count,
        gain,
    )
}

pub fn effective_load_options(
    settings: &DesktopSettings,
    explicit: NesLoadOptions,
) -> NesLoadOptions {
    explicit.with_default_mmc3_irq_variant(system_settings(settings).core.mmc3_irq_variant)
}

pub fn system_settings(settings: &DesktopSettings) -> NesSettings {
    settings
        .systems
        .get(&nerust_input_schema::SystemId::Nes)
        .map(|settings| match settings {
            SystemSettings::Nes(nes) => nes.clone(),
        })
        .unwrap_or_default()
}

pub fn filter_type(settings: &DesktopSettings) -> FilterType {
    match system_settings(settings).video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_load_options, filter_type};
    use crate::load::{NesLoadOptions, NesMmc3IrqVariant};
    use crate::settings::defaults::seed::default_desktop_settings;
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_settings::{desktop::SystemSettings, nes::NesVideoFilter};

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut settings = default_desktop_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let resolved = effective_load_options(
            &settings,
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Nec),
            },
        );

        assert_eq!(resolved.mmc3_irq_variant, Some(NesMmc3IrqVariant::Nec));
    }

    #[test]
    fn saved_nes_filter_maps_to_screen_filter_type() {
        let mut settings = default_desktop_settings();
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
}
