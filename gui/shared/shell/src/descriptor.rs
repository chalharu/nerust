use crate::emu_core::EmuCore;
use crate::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use crate::session::metrics::ConsoleMetrics;
use crate::settings::i18n::{UiText, text};
use crate::settings::nes::{build_speaker, effective_load_options, filter_type};
use nerust_contract_core::options::Mmc3IrqVariant;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
use nerust_gui_settings::shared::SystemSettings;
use nerust_input_nes::codec::{decode_input_state, encode_input_state};
use nerust_input_nes::input::NesInputState;
use nerust_input_nes::topology::input_topology_descriptor;
use nerust_input_nes_runtime::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_input_schema::{DigitalInputEvent, InputTopologyDescriptor, SystemId};
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_screen_video::{VideoFrameHandle, VideoRenderProfile};

use std::borrow::Cow;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDescriptor {
    pub system_id: SystemId,
    pub input_topology: InputTopologyDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemSettingsFieldId(pub Cow<'static, str>);

impl SystemSettingsFieldId {
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemSettingsChoiceId(pub Cow<'static, str>);

impl SystemSettingsChoiceId {
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsPageModel {
    pub fields: Arc<[SystemSettingsFieldModel]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsFieldModel {
    pub id: SystemSettingsFieldId,
    pub label: String,
    pub kind: SystemSettingsFieldKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemSettingsFieldKind {
    Choice {
        selected: SystemSettingsChoiceId,
        options: Arc<[SystemSettingsChoiceOption]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsChoiceOption {
    pub id: SystemSettingsChoiceId,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct SystemRuntimeSnapshot {
    pub metrics: ConsoleMetrics,
    pub video_frame: Option<VideoFrameHandle>,
    pub video_profile: Option<VideoRenderProfile>,
}

pub trait SystemInputAdapter: Send {
    fn apply_event(&mut self, event: DigitalInputEvent);
    fn clear(&mut self);
    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String>;
}

#[derive(Debug)]
pub(crate) struct NesAdapter {
    input: NesInputState,
    cell: Arc<NesInputCell>,
}

impl NesAdapter {
    pub(crate) fn new(cell: Arc<NesInputCell>) -> Self {
        Self {
            input: NesInputState::default(),
            cell,
        }
    }
}

impl SystemInputAdapter for NesAdapter {
    fn apply_event(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
        let frame = self.input.current_frame();
        self.cell.store(
            frame.player_one.bits(),
            frame.player_two.bits(),
            frame.microphone,
        );
    }

    fn clear(&mut self) {
        let _ = self.input.clear_current_frame();
        self.cell.store(0, 0, false);
    }

    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let frame = decode_input_state(bytes).map_err(|error| error.to_string())?;
        self.input.sync_from_frame(frame);
        self.cell.store(
            frame.player_one.bits(),
            frame.player_two.bits(),
            frame.microphone,
        );
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String> {
        encode_input_state(self.input.current_frame()).map_err(|error| error.to_string())
    }
}

const FILTER_FIELD: &str = "video.filter";
const MMC3_FIELD: &str = "core.mmc3_irq_variant";

pub fn default_system_descriptor() -> SystemDescriptor {
    SystemDescriptor {
        system_id: SystemId::Nes,
        input_topology: input_topology_descriptor(),
    }
}

pub fn default_input_topology_descriptor() -> InputTopologyDescriptor {
    input_topology_descriptor()
}

pub fn probe_nes_media(_media: &MediaObject) -> bool {
    true
}

pub fn default_nes_load_options() -> SystemLoadOptions {
    SystemLoadOptions::default()
}

pub fn resolve_nes_load_request(
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

pub fn nes_settings_page(settings: &SettingsSnapshot) -> SystemSettingsPageModel {
    let language = settings.shared.general.language;
    let current = system_settings(settings);
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

pub fn apply_nes_settings_choice(
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

pub(crate) fn create_core_and_adapter(
    settings: &SettingsSnapshot,
) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), String> {
    let speaker = build_speaker(&settings.local);
    let filter = filter_type(&settings.shared);
    let cell = Arc::new(NesInputCell::new());
    let device = NesPadDevice::new(SharedNesInputCell(cell.clone()));
    let core = EmuCore::new_gpu(
        speaker,
        filter,
        nerust_screen_video::LogicalSize {
            width: 256,
            height: 240,
        },
        Box::new(device),
    );
    let adapter = Box::new(NesAdapter::new(cell));
    Ok((core, adapter))
}

fn system_settings(settings: &SettingsSnapshot) -> NesSettings {
    settings
        .shared
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

#[cfg(test)]
mod tests {
    use super::{
        SystemSettingsChoiceId, SystemSettingsFieldId, apply_nes_settings_choice,
        default_input_topology_descriptor, nes_settings_page,
    };
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_contract_core::options::Mmc3IrqVariant;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_input_nes::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD,
        NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
    };
    use nerust_input_schema::ControlDescriptor;
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
        let descriptor = default_input_topology_descriptor();

        assert_eq!(descriptor.ports.len(), 2);
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_ONE)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_ONE_PAD
        );
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_TWO)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_TWO_FAMICOM_PAD
        );
    }

    #[test]
    fn default_descriptor_keeps_select_and_microphone_controls() {
        let descriptor = default_input_topology_descriptor();
        let player_one_controls = &descriptor
            .device(NES_DEVICE_PLAYER_ONE_PAD)
            .unwrap()
            .controls;
        let player_two_controls = &descriptor
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

    #[test]
    fn resolved_load_request_uses_saved_defaults() {
        use super::resolve_nes_load_request;
        use crate::load::SystemLoadOptions;
        let settings = snapshot();

        let resolved = resolve_nes_load_request(
            &settings,
            SystemLoadOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            },
        )
        .unwrap();

        assert_eq!(resolved.options.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));
    }

    #[test]
    fn system_page_choice_writeback_updates_snapshot() {
        let mut settings = snapshot();

        apply_nes_settings_choice(
            &mut settings,
            &SystemSettingsFieldId(Cow::Borrowed("core.mmc3_irq_variant")),
            &SystemSettingsChoiceId(Cow::Borrowed("sharp")),
        )
        .unwrap();

        let page = nes_settings_page(&settings);
        assert_eq!(page.fields.len(), 2);
    }
}
