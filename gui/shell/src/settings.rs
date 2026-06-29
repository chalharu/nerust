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

pub fn settings_view(snapshot: &SettingsSnapshot, system_id: &SystemId) -> FactorySettingsView {
    let language = match snapshot.shared.general.language {
        nerust_gui_settings::language::AppLanguage::Japanese => 1,
        nerust_gui_settings::language::AppLanguage::English => 2,
        _ => 0,
    };
    let system_config_bytes = snapshot
        .shared
        .systems
        .get(system_id)
        .map(|s| match s {
            SystemSettings::Nes(nes) => rmp_serde::to_vec(nes).unwrap_or_default(),
        })
        .unwrap_or_default();
    FactorySettingsView {
        language,
        system_config_bytes,
    }
}

fn resolve_nes_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    use nerust_gui_settings::language::AppLanguage;
    let localized = |en: &str, ja: &str| -> String {
        match language {
            AppLanguage::Japanese => ja.to_string(),
            _ => en.to_string(),
        }
    };
    match label_id {
        "nes.video.filter" => localized("Filter", "フィルター"),
        "nes.filter.none" => localized("None", "なし"),
        "nes.filter.ntsc_composite" => localized("NTSC Composite", "NTSC コンポジット"),
        "nes.filter.ntsc_svideo" => localized("NTSC S-Video", "NTSC S-ビデオ"),
        "nes.filter.ntsc_rgb" => localized("NTSC RGB", "NTSC RGB"),
        "nes.core.mmc3_irq_variant" => localized("MMC3 IRQ Variant", "MMC3 IRQ バリアント"),
        "nes.mmc3.auto" => localized("Auto", "自動"),
        "nes.mmc3.sharp" => localized("Sharp", "Sharp"),
        "nes.mmc3.nec" => localized("Nec", "Nec"),
        _ => label_id.to_string(),
    }
}

pub fn resolve_label(
    label_id: &str,
    language: nerust_gui_settings::language::AppLanguage,
) -> String {
    if label_id.starts_with("nes.") {
        return resolve_nes_label(label_id, language);
    }
    label_id.to_string()
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
    let system_id = factory.system_id();
    let mut view = settings_view(snapshot, &system_id);
    factory.apply_settings_choice(&mut view, field, choice)?;
    // Write back system config to snapshot
    if !view.system_config_bytes.is_empty()
        && let Ok(nes) = rmp_serde::from_slice::<nerust_gui_settings::nes::NesSettings>(
            &view.system_config_bytes,
        )
    {
        snapshot.shared.systems.insert(
            system_id,
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
