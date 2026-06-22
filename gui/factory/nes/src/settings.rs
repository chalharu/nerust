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
use nerust_nes_core::core_options::CoreOptions;
use nerust_nes_core::core_options::Mmc3IrqVariant;
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

fn convert_mmc3(v: nerust_gui_settings::nes::Mmc3IrqVariant) -> Mmc3IrqVariant {
    match v {
        nerust_gui_settings::nes::Mmc3IrqVariant::Sharp => Mmc3IrqVariant::Sharp,
        nerust_gui_settings::nes::Mmc3IrqVariant::Nec => Mmc3IrqVariant::Nec,
    }
}

pub(crate) fn effective_load_options(
    settings: &nerust_gui_settings::shared::DesktopSharedSettings,
    explicit: SystemLoadOptions,
) -> SystemLoadOptions {
    let saved = system_settings(settings)
        .core
        .mmc3_irq_variant
        .map(convert_mmc3);
    let explicit_val = if explicit.options_bytes.is_empty() {
        None
    } else if explicit.options_bytes == crate::MMC3_OPTION_SHARP {
        Some(Mmc3IrqVariant::Sharp)
    } else if explicit.options_bytes == crate::MMC3_OPTION_NEC {
        Some(Mmc3IrqVariant::Nec)
    } else {
        None
    };
    let core_opts = CoreOptions {
        mmc3_irq_variant: explicit_val.or(saved),
    };
    SystemLoadOptions {
        options_bytes: core_opts.into_bytes(),
    }
}

pub(crate) fn resolve_nes_load_request(
    settings: &SettingsSnapshot,
    options: SystemLoadOptions,
) -> Result<ResolvedLoadRequest, String> {
    let resolved = effective_load_options(&settings.shared, options);
    let core_opts = CoreOptions::from_bytes(&resolved.options_bytes)
        .map_err(|e| format!("failed to decode core options: {e}"))?;
    Ok(ResolvedLoadRequest {
        system_id: SystemId::Nes,
        options: resolved,
        core_options_bytes: core_opts.into_bytes(),
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
                            Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp) => "sharp",
                            Some(nerust_gui_settings::nes::Mmc3IrqVariant::Nec) => "nec",
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
                "sharp" => Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp),
                "nec" => Some(nerust_gui_settings::nes::Mmc3IrqVariant::Nec),
                other => return Err(format!("unsupported mmc3 choice: {other}")),
            };
            Ok(())
        }
        other => Err(format!("unsupported system settings field: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_nes_settings_choice, effective_load_options, filter_type, nes_settings_page,
        resolve_nes_load_request,
    };
    use crate::NesFactory;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::{nes::NesVideoFilter, shared::SystemSettings};
    use nerust_gui_shell::factory::CoreFactory;
    use nerust_gui_shell::load::SystemLoadOptions;
    use nerust_gui_shell::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_input_nes_runtime::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD,
        NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
    };
    use nerust_input_schema::ControlDescriptor;
    use nerust_nes_core::core_options::CoreOptions;
    use nerust_nes_core::core_options::Mmc3IrqVariant;
    use std::borrow::Cow;

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
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
        let settings = snapshot();

        let resolved = resolve_nes_load_request(&settings, nec_options()).unwrap();

        let core_opts =
            CoreOptions::from_bytes(&resolved.core_options_bytes).expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn system_page_choice_writeback_updates_snapshot() {
        let mut settings = snapshot();

        apply_nes_settings_choice(
            &mut settings,
            &nerust_gui_shell::descriptor::SystemSettingsFieldId(Cow::Borrowed(
                "core.mmc3_irq_variant",
            )),
            &nerust_gui_shell::descriptor::SystemSettingsChoiceId(Cow::Borrowed("sharp")),
        )
        .unwrap();

        let page = nes_settings_page(&settings);
        assert_eq!(page.fields.len(), 2);
    }

    #[test]
    fn explicit_load_options_win_over_saved_defaults() {
        let mut settings = default_shared_settings();
        let SystemSettings::Nes(nes) = settings
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap();
        nes.core.mmc3_irq_variant = Some(nerust_gui_settings::nes::Mmc3IrqVariant::Sharp);

        let resolved = effective_load_options(&settings, nec_options());

        let core_opts =
            CoreOptions::from_bytes(&resolved.options_bytes).expect("valid core options");
        assert_eq!(core_opts.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
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
            nerust_screen_video::FilterType::NtscSVideo
        ));
    }
}
