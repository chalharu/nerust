pub mod bindings;
pub mod defaults;
pub mod editor;
pub mod i18n;

use nerust_core_traits::SystemId;
use nerust_core_traits::audio::{AudioBackend, AudioBackendRegistry, GainBackend};
use nerust_core_traits::factory::settings::FactorySettingsView;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::local::{HostBackendLocalSettings, ScalingMode};
use nerust_gui_settings::shared::SystemSettings;

pub fn settings_view(snapshot: &SettingsSnapshot) -> FactorySettingsView {
    let language = match snapshot.shared.general.language {
        nerust_gui_settings::language::AppLanguage::Japanese => 1,
        nerust_gui_settings::language::AppLanguage::English => 2,
        _ => 0,
    };
    let system_config_bytes = snapshot
        .shared
        .systems
        .get(&SystemId::new("nes"))
        .map(|s| match s {
            SystemSettings::Nes(nes) => rmp_serde::to_vec(nes).unwrap_or_default(),
        })
        .unwrap_or_default();
    FactorySettingsView {
        language,
        system_config_bytes,
    }
}

pub fn resolve_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    use nerust_gui_settings::language::AppLanguage;
    let label = |id: &str, map: &[(&str, &str, &str)]| -> String {
        for &(en, ja, id_match) in map {
            if id == id_match {
                return match language {
                    AppLanguage::Japanese => ja.to_string(),
                    _ => en.to_string(),
                };
            }
        }
        id.to_string()
    };
    label(
        label_id,
        &[
            ("Filter", "フィルター", "nes.video.filter"),
            ("None", "なし", "nes.filter.none"),
            (
                "NTSC Composite",
                "NTSC コンポジット",
                "nes.filter.ntsc_composite",
            ),
            ("NTSC S-Video", "NTSC S-ビデオ", "nes.filter.ntsc_svideo"),
            ("NTSC RGB", "NTSC RGB", "nes.filter.ntsc_rgb"),
            (
                "MMC3 IRQ Variant",
                "MMC3 IRQ バリアント",
                "nes.core.mmc3_irq_variant",
            ),
            ("Auto", "自動", "nes.mmc3.auto"),
            ("Sharp", "Sharp", "nes.mmc3.sharp"),
            ("Nec", "Nec", "nes.mmc3.nec"),
        ],
    )
}

pub fn build_speaker(
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

pub fn apply_settings_choice(
    factory: &dyn nerust_core_traits::factory::CoreFactory,
    snapshot: &mut SettingsSnapshot,
    field: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
    choice: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
) -> Result<(), nerust_core_traits::factory::FactoryError> {
    let mut view = settings_view(snapshot);
    factory.apply_settings_choice(&mut view, field, choice)?;
    // Write back system config to snapshot
    if !view.system_config_bytes.is_empty()
        && let Ok(nes) = rmp_serde::from_slice::<nerust_gui_settings::nes::NesSettings>(
            &view.system_config_bytes,
        )
    {
        snapshot.shared.systems.insert(
            nerust_core_traits::SystemId::new("nes"),
            nerust_gui_settings::shared::SystemSettings::Nes(nes),
        );
    }
    Ok(())
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
    use nerust_core_traits::audio::{AudioBackend, NullAudio};
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
