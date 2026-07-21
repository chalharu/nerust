use std::{borrow::Cow, sync::Arc};

use nerust_core_traits::factory::{
    FactoryError,
    descriptor::{
        SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
        SystemSettingsFieldKind, SystemSettingsFieldModel, SystemSettingsPageModel,
    },
    load::{DynSystemLoadOptions, DynSystemLoadOptionsExt, ResolvedLoadRequest},
    settings::{FactorySettingsView, Language},
};
use nerust_nes_core::core_options::{CoreOptions, Mmc3IrqVariant};
use nerust_nes_settings::{NesSettings, NesVideoFilter};
use nerust_render_traits::filter::FilterType;
use nerust_settings_traits::SystemSettings;

use crate::CommandLineOptions;

pub(crate) fn filter_type_from_bytes(settings: Option<&dyn SystemSettings>) -> FilterType {
    let default_settings = NesSettings::default();
    let nes_settings = settings
        .and_then(|s| s.downcast_ref())
        .unwrap_or(&default_settings);
    match nes_settings.video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
    }
}

pub(crate) fn nes_settings_page(view: &FactorySettingsView) -> SystemSettingsPageModel {
    nes_settings_page_inner(
        view.system_config
            .as_deref()
            .and_then(|s| s.downcast_ref())
            .unwrap_or(&NesSettings::default()),
    )
}

fn nes_settings_page_inner(current: &NesSettings) -> SystemSettingsPageModel {
    SystemSettingsPageModel {
        fields: Arc::from([
            SystemSettingsFieldModel {
                id: SystemSettingsFieldId(Cow::Borrowed(FILTER_FIELD)),
                label_id: "nes.video.filter",
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
                            label_id: "nes.filter.none",
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_composite")),
                            label_id: "nes.filter.ntsc_composite",
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_svideo")),
                            label_id: "nes.filter.ntsc_svideo",
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("ntsc_rgb")),
                            label_id: "nes.filter.ntsc_rgb",
                        },
                    ]),
                },
            },
            SystemSettingsFieldModel {
                id: SystemSettingsFieldId(Cow::Borrowed(MMC3_FIELD)),
                label_id: "nes.core.mmc3_irq_variant",
                kind: SystemSettingsFieldKind::Choice {
                    selected: SystemSettingsChoiceId(Cow::Borrowed(
                        match current.core.mmc3_irq_variant {
                            Some(nerust_nes_settings::Mmc3IrqVariant::Sharp) => "sharp",
                            Some(nerust_nes_settings::Mmc3IrqVariant::Nec) => "nec",
                            None => "auto",
                        },
                    )),
                    options: Arc::from([
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("auto")),
                            label_id: "nes.mmc3.auto",
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("sharp")),
                            label_id: "nes.mmc3.sharp",
                        },
                        SystemSettingsChoiceOption {
                            id: SystemSettingsChoiceId(Cow::Borrowed("nec")),
                            label_id: "nes.mmc3.nec",
                        },
                    ]),
                },
            },
        ]),
    }
}

const FILTER_FIELD: &str = "video.filter";
const MMC3_FIELD: &str = "core.mmc3_irq_variant";

fn convert_mmc3(v: nerust_nes_settings::Mmc3IrqVariant) -> Mmc3IrqVariant {
    match v {
        nerust_nes_settings::Mmc3IrqVariant::Sharp => Mmc3IrqVariant::Sharp,
        nerust_nes_settings::Mmc3IrqVariant::Nec => Mmc3IrqVariant::Nec,
    }
}

pub(crate) fn resolve_nes_load_request_inner(
    nes: &NesSettings,
    _language: &Language,
    options: Box<dyn DynSystemLoadOptions>,
) -> Result<ResolvedLoadRequest, FactoryError> {
    let saved = nes.core.mmc3_irq_variant.map(convert_mmc3);
    let options = options
        .into_inner::<CommandLineOptions>()
        .map_err(|_| FactoryError::Resolve("failed to downcast load options".to_string()))?;
    let explicit_val = options.mmc3_irq_variant.map(Mmc3IrqVariant::from);
    let core_opts = CoreOptions {
        mmc3_irq_variant: explicit_val.or(saved),
    };
    Ok(ResolvedLoadRequest {
        options: core_opts.into(),
    })
}

pub(crate) fn apply_nes_settings_choice_inner(
    s: &mut NesSettings,
    field: &SystemSettingsFieldId,
    choice: &SystemSettingsChoiceId,
) -> Result<(), FactoryError> {
    match field.as_str() {
        FILTER_FIELD => {
            s.video.filter = match choice.as_str() {
                "none" => NesVideoFilter::None,
                "ntsc_composite" => NesVideoFilter::NtscComposite,
                "ntsc_svideo" => NesVideoFilter::NtscSVideo,
                "ntsc_rgb" => NesVideoFilter::NtscRgb,
                other => return Err(FactoryError::InvalidChoice(other.to_string())),
            };
            Ok(())
        }
        MMC3_FIELD => {
            s.core.mmc3_irq_variant = match choice.as_str() {
                "sharp" => Some(nerust_nes_settings::Mmc3IrqVariant::Sharp),
                "nec" => Some(nerust_nes_settings::Mmc3IrqVariant::Nec),
                "auto" => None,
                other => return Err(FactoryError::InvalidChoice(other.to_string())),
            };
            Ok(())
        }
        _ => Err(FactoryError::InvalidChoice(field.as_str().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use nerust_core_traits::{
        DynCoreOptionsExt,
        factory::{
            descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
            load::DynSystemLoadOptions,
            settings::{FactorySettingsView, Language},
        },
    };
    use nerust_nes_core::core_options::{CoreOptions, Mmc3IrqVariant};
    use nerust_nes_settings::{NesSettings, NesVideoFilter};
    use nerust_render_traits::filter::FilterType;

    use crate::CommandLineOptions;

    use super::{
        apply_nes_settings_choice_inner, filter_type_from_bytes, nes_settings_page,
        resolve_nes_load_request_inner,
    };

    fn test_view() -> FactorySettingsView {
        FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(NesSettings::default())),
        }
    }

    fn nec_options() -> Box<dyn DynSystemLoadOptions> {
        CommandLineOptions {
            mmc3_irq_variant: Some(crate::Mmc3IrqVariant::Nec),
        }
        .into()
    }

    #[test]
    fn resolved_load_request_uses_saved_defaults() {
        let view = test_view();
        let nes = view
            .system_config
            .as_deref()
            .unwrap()
            .downcast_ref::<NesSettings>()
            .unwrap();
        let resolved =
            resolve_nes_load_request_inner(nes, &Language::SystemDefault, nec_options()).unwrap();

        let core_opts = &resolved
            .options
            .into_inner::<CoreOptions>()
            .expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn system_page_choice_writeback_updates_snapshot() {
        let mut nes = NesSettings::default();
        apply_nes_settings_choice_inner(
            &mut nes,
            &SystemSettingsFieldId(Cow::Borrowed("core.mmc3_irq_variant")),
            &SystemSettingsChoiceId(Cow::Borrowed("sharp")),
        )
        .unwrap();

        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config: Some(Box::new(nes)),
        };
        let page = nes_settings_page(&view);
        assert_eq!(page.fields.len(), 2);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut nes = NesSettings::default();
        nes.core.mmc3_irq_variant = Some(nerust_nes_settings::Mmc3IrqVariant::Sharp);

        let resolved =
            resolve_nes_load_request_inner(&nes, &Language::SystemDefault, nec_options()).unwrap();

        let core_opts = &resolved
            .options
            .into_inner::<CoreOptions>()
            .expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn saved_nes_filter_maps_to_screen_filter_type() {
        let mut nes = NesSettings::default();
        nes.video.filter = NesVideoFilter::NtscSVideo;

        assert!(matches!(
            filter_type_from_bytes(Some(&nes)),
            FilterType::NtscSVideo
        ));
    }

    #[test]
    fn mmc3_irq_variant_conversion_covers_all_variants() {
        use nerust_nes_core::core_options::Mmc3IrqVariant as CoreVariant;
        use nerust_nes_settings::Mmc3IrqVariant as SettingsVariant;

        assert_eq!(
            super::convert_mmc3(SettingsVariant::Sharp),
            CoreVariant::Sharp
        );
        assert_eq!(super::convert_mmc3(SettingsVariant::Nec), CoreVariant::Nec);
    }

    #[test]
    fn mmc3_irq_variant_round_trips_via_load_options() {
        let nes = NesSettings::default(); // defaults
        let resolved = resolve_nes_load_request_inner(
            &nes,
            &Language::SystemDefault,
            CommandLineOptions {
                mmc3_irq_variant: Some(crate::Mmc3IrqVariant::Sharp),
            }
            .into(),
        )
        .unwrap();

        let core_opts = &resolved.options.into_inner::<CoreOptions>().unwrap();
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Sharp));

        let resolved = resolve_nes_load_request_inner(
            &nes,
            &Language::SystemDefault,
            CommandLineOptions {
                mmc3_irq_variant: Some(crate::Mmc3IrqVariant::Nec),
            }
            .into(),
        )
        .unwrap();
        let core_opts = &resolved.options.into_inner::<CoreOptions>().unwrap();
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }
}
