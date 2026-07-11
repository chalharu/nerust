use std::{borrow::Cow, sync::Arc};

use nerust_core_traits::factory::FactoryError;
use nerust_core_traits::factory::descriptor::{
    SystemSettingsChoiceId, SystemSettingsChoiceOption, SystemSettingsFieldId,
    SystemSettingsFieldKind, SystemSettingsFieldModel, SystemSettingsPageModel,
};
use nerust_core_traits::factory::load::{ResolvedLoadRequest, SystemLoadOptions};
use nerust_core_traits::factory::settings::{FactorySettingsView, Language};
use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
use nerust_nes_core::core_options::{CoreOptions, Mmc3IrqVariant};
use nerust_render_base::filter::FilterType;

pub(crate) fn deserialize_settings(bytes: &[u8]) -> NesSettings {
    if bytes.is_empty() {
        return NesSettings::default();
    }
    rmp_serde::from_slice(bytes).unwrap_or_default()
}

pub(crate) fn serialize_settings(s: &NesSettings) -> Vec<u8> {
    rmp_serde::to_vec(s).unwrap_or_default()
}

pub(crate) fn filter_type_from_bytes(bytes: &[u8]) -> FilterType {
    let settings = deserialize_settings(bytes);
    match settings.video.filter {
        NesVideoFilter::None => FilterType::None,
        NesVideoFilter::NtscComposite => FilterType::NtscComposite,
        NesVideoFilter::NtscSVideo => FilterType::NtscSVideo,
        NesVideoFilter::NtscRgb => FilterType::NtscRGB,
    }
}

pub(crate) fn nes_settings_page(_view: &FactorySettingsView) -> SystemSettingsPageModel {
    let current = deserialize_settings(&_view.system_config_bytes);
    nes_settings_page_inner(&current)
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
                            Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp) => "sharp",
                            Some(nerust_gui_settings::nes::Mmc3IrqVariant::Nec) => "nec",
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

fn convert_mmc3(v: nerust_gui_settings::nes::Mmc3IrqVariant) -> Mmc3IrqVariant {
    match v {
        nerust_gui_settings::nes::Mmc3IrqVariant::Sharp => Mmc3IrqVariant::Sharp,
        nerust_gui_settings::nes::Mmc3IrqVariant::Nec => Mmc3IrqVariant::Nec,
    }
}

pub(crate) fn resolve_nes_load_request_inner(
    nes: &NesSettings,
    _language: &Language,
    options: SystemLoadOptions,
) -> Result<ResolvedLoadRequest, FactoryError> {
    let saved = nes.core.mmc3_irq_variant.map(convert_mmc3);
    let explicit_val = if options.options_bytes.is_empty() {
        None
    } else if options.options_bytes == crate::MMC3_OPTION_SHARP {
        Some(Mmc3IrqVariant::Sharp)
    } else if options.options_bytes == crate::MMC3_OPTION_NEC {
        Some(Mmc3IrqVariant::Nec)
    } else {
        None
    };
    let core_opts = CoreOptions {
        mmc3_irq_variant: explicit_val.or(saved),
    };
    let resolved = SystemLoadOptions {
        options_bytes: core_opts.into_bytes(),
    };
    let core_opts = CoreOptions::from_bytes(&resolved.options_bytes)
        .map_err(|e| FactoryError::Resolve(format!("failed to decode core options: {e}")))?;
    Ok(ResolvedLoadRequest {
        options: resolved,
        core_options_bytes: core_opts.into_bytes(),
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
                "sharp" => Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp),
                "nec" => Some(nerust_gui_settings::nes::Mmc3IrqVariant::Nec),
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

    use nerust_core_traits::factory::CoreFactory;
    use nerust_core_traits::factory::descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId};
    use nerust_core_traits::factory::load::SystemLoadOptions;
    use nerust_core_traits::factory::settings::{FactorySettingsView, Language};

    use nerust_gui_settings::nes::NesVideoFilter;

    use nerust_input_traits::ControlDescriptor;
    use nerust_nes_controller::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD,
        NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
    };
    use nerust_nes_core::core_options::{CoreOptions, Mmc3IrqVariant};
    use nerust_render_base::filter::FilterType;

    use super::{
        apply_nes_settings_choice_inner, filter_type_from_bytes, nes_settings_page,
        resolve_nes_load_request_inner,
    };
    use crate::NesFactory;

    fn test_view() -> FactorySettingsView {
        FactorySettingsView {
            language: Language::SystemDefault,
            system_config_bytes: vec![],
        }
    }

    #[test]
    fn default_descriptor_reports_distinct_player_devices() {
        let factory = NesFactory;
        let descriptor = factory.system_descriptor();

        assert_eq!(descriptor.input_topology.ports.len(), 2);
        assert_eq!(
            descriptor
                .input_topology
                .attachment(NES_ATTACHMENT_PLAYER_ONE)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_ONE_PAD
        );
        assert_eq!(
            descriptor
                .input_topology
                .attachment(NES_ATTACHMENT_PLAYER_TWO)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_TWO_FAMICOM_PAD
        );
    }

    #[test]
    fn default_descriptor_keeps_select_and_microphone_controls() {
        let factory = NesFactory;
        let descriptor = factory.system_descriptor();
        let player_one_controls = &descriptor
            .input_topology
            .device(NES_DEVICE_PLAYER_ONE_PAD)
            .unwrap()
            .controls;
        let player_two_controls = &descriptor
            .input_topology
            .device(NES_DEVICE_PLAYER_TWO_FAMICOM_PAD)
            .unwrap()
            .controls;

        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_A
            )
        }));
        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_SELECT
            )
        }));
        assert!(player_two_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == FAMICOM_P2_CONTROL_MICROPHONE
            )
        }));
    }

    fn nec_options() -> SystemLoadOptions {
        SystemLoadOptions {
            options_bytes: b"nec".to_vec(),
        }
    }

    #[test]
    fn resolved_load_request_uses_saved_defaults() {
        let view = test_view();
        let nes = super::deserialize_settings(&view.system_config_bytes);
        let resolved =
            resolve_nes_load_request_inner(&nes, &Language::SystemDefault, nec_options()).unwrap();

        let core_opts =
            CoreOptions::from_bytes(&resolved.core_options_bytes).expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn system_page_choice_writeback_updates_snapshot() {
        let mut nes = super::deserialize_settings(&[]);
        apply_nes_settings_choice_inner(
            &mut nes,
            &SystemSettingsFieldId(Cow::Borrowed("core.mmc3_irq_variant")),
            &SystemSettingsChoiceId(Cow::Borrowed("sharp")),
        )
        .unwrap();

        let view = FactorySettingsView {
            language: Language::SystemDefault,
            system_config_bytes: super::serialize_settings(&nes),
        };
        let page = nes_settings_page(&view);
        assert_eq!(page.fields.len(), 2);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut nes = super::deserialize_settings(&[]);
        nes.core.mmc3_irq_variant = Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp);

        let resolved =
            resolve_nes_load_request_inner(&nes, &Language::SystemDefault, nec_options()).unwrap();

        let core_opts =
            CoreOptions::from_bytes(&resolved.core_options_bytes).expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn saved_nes_filter_maps_to_screen_filter_type() {
        let mut nes = super::deserialize_settings(&[]);
        nes.video.filter = NesVideoFilter::NtscSVideo;
        let bytes = super::serialize_settings(&nes);

        assert!(matches!(
            filter_type_from_bytes(&bytes),
            FilterType::NtscSVideo
        ));
    }

    #[test]
    fn mmc3_irq_variant_conversion_covers_all_variants() {
        use nerust_gui_settings::nes::Mmc3IrqVariant as SettingsVariant;
        use nerust_nes_core::core_options::Mmc3IrqVariant as CoreVariant;

        assert_eq!(
            super::convert_mmc3(SettingsVariant::Sharp),
            CoreVariant::Sharp
        );
        assert_eq!(super::convert_mmc3(SettingsVariant::Nec), CoreVariant::Nec);
    }

    #[test]
    fn mmc3_irq_variant_round_trips_via_load_options() {
        let nes = super::deserialize_settings(&[]); // defaults
        let resolved = resolve_nes_load_request_inner(
            &nes,
            &Language::SystemDefault,
            SystemLoadOptions {
                options_bytes: crate::MMC3_OPTION_SHARP.to_vec(),
            },
        )
        .unwrap();
        let core_opts = CoreOptions::from_bytes(&resolved.core_options_bytes).unwrap();
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Sharp));

        let resolved = resolve_nes_load_request_inner(
            &nes,
            &Language::SystemDefault,
            SystemLoadOptions {
                options_bytes: crate::MMC3_OPTION_NEC.to_vec(),
            },
        )
        .unwrap();
        let core_opts = CoreOptions::from_bytes(&resolved.core_options_bytes).unwrap();
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }
}
