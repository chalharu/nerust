use nerust_contract_core::options::Mmc3IrqVariant;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
use nerust_gui_settings::shared::SystemSettings;
use nerust_gui_shell::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
    SystemSettingsFieldKind, SystemSettingsFieldModel, SystemSettingsPageModel,
};
use nerust_gui_shell::load::{ResolvedLoadRequest, SystemLoadOptions};
use nerust_gui_shell::settings::i18n::{UiText, text};
use nerust_input_schema::SystemId;
use nerust_screen_video::FilterType;
use std::borrow::Cow;
use std::sync::Arc;

const FILTER_FIELD: &str = "video.filter";
const MMC3_FIELD: &str = "core.mmc3_irq_variant";

pub(crate) fn filter_type(
    settings: &nerust_gui_settings::shared::DesktopSharedSettings,
) -> FilterType {
    match system_settings(settings).video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
    }
}

fn system_settings(settings: &nerust_gui_settings::shared::DesktopSharedSettings) -> NesSettings {
    settings
        .systems
        .get(&SystemId::Nes)
        .map(|settings| match settings {
            SystemSettings::Nes(nes) => nes.clone(),
        })
        .unwrap_or_default()
}

fn system_settings_mut(settings: &mut SettingsSnapshot) -> &mut NesSettings {
    let current = settings
        .shared
        .systems
        .entry(SystemId::Nes)
        .or_insert_with(|| SystemSettings::Nes(NesSettings::default()));
    match current {
        SystemSettings::Nes(nes) => nes,
    }
}

pub(crate) fn effective_load_options(
    settings: &nerust_gui_settings::shared::DesktopSharedSettings,
    explicit: SystemLoadOptions,
) -> SystemLoadOptions {
    explicit.with_mmc3_irq_variant(system_settings(settings).core.mmc3_irq_variant)
}

pub(crate) fn resolve_nes_load_request(
    settings: &SettingsSnapshot,
    options: SystemLoadOptions,
) -> Result<ResolvedLoadRequest, String> {
    let resolved = effective_load_options(&settings.shared, options);
    Ok(ResolvedLoadRequest {
        system_id: SystemId::Nes,
        options: resolved,
        core_options: resolved.into_core_options(),
    })
}

pub(crate) fn nes_settings_page(settings: &SettingsSnapshot) -> SystemSettingsPageModel {
    let language = settings.shared.general.language;
    let current = system_settings(&settings.shared);
    SystemSettingsPageModel {
        fields: Arc::from([
            SystemSettingsFieldModel {
                id: SystemSettingsFieldId(Cow::Borrowed(FILTER_FIELD)),
                label: text(language, UiText::Filter).to_string(),
                kind: SystemSettingsFieldKind::Choice {
                    selected: SystemSettingsChoiceId(Cow::Borrowed(match current.video.filter {
                        NesVideoFilter::None => "none",
                        NesVideoFilter::NtscComposite => "ntsc_composite",
                        NesVideoFilter::NtscSVideo => "ntsc_svideo",
                        NesVideoFilter::NtscRgb => "ntsc_rgb",
                    })),
                    options: Arc::from([
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("none")),
                            label: text(language, UiText::None).to_string(),
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_composite")),
                            label: text(language, UiText::NtscComposite).to_string(),
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_svideo")),
                            label: text(language, UiText::NtscSVideo).to_string(),
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_rgb")),
                            label: text(language, UiText::NtscRgb).to_string(),
                        },
                    ]),
                },
            },
            SystemSettingsFieldModel {
                id: SystemSettingsFieldId(Cow::Borrowed(MMC3_FIELD)),
                label: text(language, UiText::Mmc3IrqVariant).to_string(),
                kind: SystemSettingsFieldKind::Choice {
                    selected: SystemSettingsChoiceId(Cow::Borrowed(
                        match current.core.mmc3_irq_variant {
                            None => "auto",
                            Some(Mmc3IrqVariant::Sharp) => "sharp",
                            Some(Mmc3IrqVariant::Nec) => "nec",
                        },
                    )),
                    options: Arc::from([
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("auto")),
                            label: text(language, UiText::Auto).to_string(),
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("sharp")),
                            label: text(language, UiText::Sharp).to_string(),
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("nec")),
                            label: text(language, UiText::Nec).to_string(),
                        },
                    ]),
                },
            },
        ]),
    }
}

pub(crate) fn apply_nes_settings_choice(
    settings: &mut SettingsSnapshot,
    field: &SystemSettingsFieldId,
    choice: &SystemSettingsChoiceId,
) -> Result<(), String> {
    let current = system_settings_mut(settings);
    match field.as_str() {
        FILTER_FIELD => {
            current.video.filter = match choice.as_str() {
                "none" => NesVideoFilter::None,
                "ntsc_composite" => NesVideoFilter::NtscComposite,
                "ntsc_svideo" => NesVideoFilter::NtscSVideo,
                "ntsc_rgb" => NesVideoFilter::NtscRgb,
                other => return Err(format!("unsupported filter choice: {other}")),
            };
            Ok(())
        }
        MMC3_FIELD => {
            current.core.mmc3_irq_variant = match choice.as_str() {
                "auto" => None,
                "sharp" => Some(Mmc3IrqVariant::Sharp),
                "nec" => Some(Mmc3IrqVariant::Nec),
                other => return Err(format!("unsupported mmc3 choice: {other}")),
            };
            Ok(())
        }
        other => Err(format!("unsupported system settings field: {other}")),
    }
}
