use crate::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use crate::settings::i18n::{UiText, text};
use crate::settings::nes::{build_screen_buffer, effective_load_options};
use nerust_console::ConsoleMetrics;
use nerust_console::state::RuntimeStateExport;
use nerust_console::video::{VideoFrameHandle, VideoRenderProfile};
use nerust_contract_core::options::Mmc3IrqVariant;
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_contract_core::{ConsoleCore, CoreConfig, EmuCommand};
use nerust_contract_emuthread::EmuThread;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsSnapshot};
use nerust_gui_settings::nes::{NesSettings, NesVideoFilter};
use nerust_gui_settings::shared::SystemSettings;
use nerust_input_nes::codec::{decode_input_state, encode_input_state};
use nerust_input_nes::input::NesInputState;
use nerust_input_nes::topology::input_topology_descriptor;
use nerust_input_schema::{DigitalInputEvent, InputTopologyDescriptor, SystemId};
use nerust_nes_console::NesConsoleCore;
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDescriptor {
    pub system_id: SystemId,
    pub input_topology: InputTopologyDescriptor,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeHostServices {
    pub host_backend: HostBackendIdentity,
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

pub trait SystemDefinition: Send + Sync {
    fn descriptor(&self) -> SystemDescriptor;
    fn probe_media(&self, media: &MediaObject) -> bool;
    fn default_load_options(&self) -> SystemLoadOptions;
    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, String>;
    fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel;
    fn apply_settings_choice(
        &self,
        settings: &mut SettingsSnapshot,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), String>;
    fn create_input_adapter(&self, settings: &SettingsSnapshot) -> Box<dyn SystemInputAdapter>;
    fn create_runtime(
        &self,
        host: &RuntimeHostServices,
        settings: &SettingsSnapshot,
    ) -> Result<Box<dyn SystemRuntime>, String>;
}

pub trait SystemInputAdapter: Send {
    fn digital_event_from_persisted(
        &self,
        attachment: &str,
        control: &str,
        pressed: bool,
    ) -> Option<DigitalInputEvent>;
    fn apply_event(&mut self, event: DigitalInputEvent);
    fn clear(&mut self);
    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String>;
}

pub trait SystemRuntime: Send {
    fn snapshot(&self) -> SystemRuntimeSnapshot;
    fn load(&mut self, media: &MediaObject, request: &ResolvedLoadRequest) -> Result<(), String>;
    fn unload(&mut self) -> Result<bool, String>;
    fn reset(&self) -> Result<(), String>;
    fn pause(&mut self);
    fn resume(&mut self);
    fn apply_input_state(&mut self, bytes: Vec<u8>) -> Result<(), String>;
    fn current_input_state(&self) -> Result<Vec<u8>, String>;
    fn export_state(&self) -> Result<RuntimeStateExport, String>;
    fn import_state(&mut self, state_blob: &[u8]) -> Result<(), String>;
    fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, String>;
    fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), String>;
    fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity>;

    /// Provide access to the current frame buffer without allocating a per-frame copy.
    /// The closure is invoked while holding a read lock on the shared frame buffer.
    /// Implementations should call the closure synchronously and not retain the byte slice.
    fn with_frame_buffer(&self, f: &mut dyn FnMut(&[u8])) {
        // Default implementation: take a snapshot and, if available, invoke the closure
        // with the frame bytes while the VideoFrameHandle is still in scope.
        let snapshot = self.snapshot();
        if let Some(frame) = snapshot.video_frame {
            f(frame.bytes());
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NesSystemDefinition;

#[derive(Debug, Default)]
struct NesAdapter {
    input: NesInputState,
}

struct NesRuntime {
    emu: EmuThread<NesConsoleCore>,
    video_profile: VideoRenderProfile,
}

const FILTER_FIELD: &str = "video.filter";
const MMC3_FIELD: &str = "core.mmc3_irq_variant";

pub fn default_system_definition() -> Box<dyn SystemDefinition> {
    Box::new(NesSystemDefinition)
}

pub fn default_input_topology_descriptor() -> InputTopologyDescriptor {
    NesSystemDefinition.descriptor().input_topology
}

pub fn default_system_settings_page_model(settings: &SettingsSnapshot) -> SystemSettingsPageModel {
    NesSystemDefinition.settings_page(settings)
}

pub fn apply_default_system_settings_choice(
    settings: &mut SettingsSnapshot,
    field: &SystemSettingsFieldId,
    choice: &SystemSettingsChoiceId,
) -> Result<(), String> {
    NesSystemDefinition.apply_settings_choice(settings, field, choice)
}

impl SystemDefinition for NesSystemDefinition {
    fn descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            system_id: SystemId::Nes,
            input_topology: input_topology_descriptor(),
        }
    }

    fn probe_media(&self, _media: &MediaObject) -> bool {
        true
    }

    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }

    fn resolve_load_request(
        &self,
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

    fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        let language = settings.shared.general.language;
        let current = system_settings(settings);
        SystemSettingsPageModel {
            fields: Arc::from([
                SystemSettingsFieldModel {
                    id: SystemSettingsFieldId(Cow::Borrowed(FILTER_FIELD)),
                    label: text(language, UiText::Filter).to_string(),
                    kind: SystemSettingsFieldKind::Choice {
                        selected: SystemSettingsChoiceId(Cow::Borrowed(
                            match current.video.filter {
                                NesVideoFilter::None => "none",
                                NesVideoFilter::NtscComposite => "ntsc_composite",
                                NesVideoFilter::NtscSVideo => "ntsc_svideo",
                                NesVideoFilter::NtscRgb => "ntsc_rgb",
                            },
                        )),
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

    fn apply_settings_choice(
        &self,
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

    fn create_input_adapter(&self, _settings: &SettingsSnapshot) -> Box<dyn SystemInputAdapter> {
        Box::new(NesAdapter::default())
    }

    fn create_runtime(
        &self,
        _host: &RuntimeHostServices,
        settings: &SettingsSnapshot,
    ) -> Result<Box<dyn SystemRuntime>, String> {
        let screen = build_screen_buffer(&settings.shared);
        let core = NesConsoleCore::new(screen);
        Ok(Box::new(NesRuntime {
            emu: EmuThread::spawn(core),
            video_profile: VideoRenderProfile {
                source_logical_size: LogicalSize {
                    width: 256,
                    height: 240,
                },
                logical_size: LogicalSize {
                    width: 256,
                    height: 240,
                },
                physical_size: PhysicalSize {
                    width: 512.0,
                    height: 480.0,
                },
            },
        }))
    }
}

impl SystemInputAdapter for NesAdapter {
    fn digital_event_from_persisted(
        &self,
        attachment: &str,
        control: &str,
        pressed: bool,
    ) -> Option<DigitalInputEvent> {
        nerust_input_nes::input::persisted::digital_event_from_persisted_ids(
            attachment, control, pressed,
        )
    }

    fn apply_event(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
    }

    fn clear(&mut self) {
        let _ = self.input.clear_current_frame();
    }

    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let frame = decode_input_state(bytes).map_err(|error| error.to_string())?;
        self.input.sync_from_frame(frame);
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String> {
        encode_input_state(self.input.current_frame()).map_err(|error| error.to_string())
    }
}

impl SystemRuntime for NesRuntime {
    fn snapshot(&self) -> SystemRuntimeSnapshot {
        let frame = self.emu.wait_frame().ok().map(|result| {
            let stride = 256 * 4;
            let len = stride * 240;
            let data: Arc<[u8]> = if result.slot_data.len() >= len {
                Arc::from(&result.slot_data[..len])
            } else {
                vec![0u8; len].into()
            };
            VideoFrameHandle {
                width: 256,
                height: 240,
                stride_bytes: stride,
                bytes: data,
            }
        });
        let fps = self
            .emu
            .last_fps()
            .load(std::sync::atomic::Ordering::Relaxed) as f32
            / 100.0;
        SystemRuntimeSnapshot {
            metrics: ConsoleMetrics {
                frame_counter: 0,
                emulation_fps: fps,
                speed_multiplier: if fps > 0.0 { fps / 60.0 } else { 0.0 },
                loaded: true,
                paused: false,
            },
            video_frame: frame,
            video_profile: Some(self.video_profile.clone()),
        }
    }

    fn load(&mut self, media: &MediaObject, _request: &ResolvedLoadRequest) -> Result<(), String> {
        let screen =
            build_screen_buffer(&crate::settings::defaults::seed::default_shared_settings());
        let mut core = NesConsoleCore::new(screen);
        core.load(
            &media.bytes,
            &CoreConfig {
                region: None,
                bios_paths: std::collections::HashMap::new(),
                controllers: std::collections::HashMap::new(),
            },
        )
        .map_err(|e| e.to_string())?;
        self.emu.send(EmuCommand::Quit).ok();
        self.emu = EmuThread::spawn(core);
        Ok(())
    }

    fn unload(&mut self) -> Result<bool, String> {
        self.emu.send(EmuCommand::Quit).ok();
        let screen =
            build_screen_buffer(&crate::settings::defaults::seed::default_shared_settings());
        let core = NesConsoleCore::new(screen);
        self.emu = EmuThread::spawn(core);
        Ok(true)
    }

    fn reset(&self) -> Result<(), String> {
        self.emu.send(EmuCommand::Reset).map_err(|e| e.to_string())
    }

    fn pause(&mut self) {
        self.emu.send(EmuCommand::Pause).ok();
    }

    fn resume(&mut self) {
        self.emu.send(EmuCommand::Resume).ok();
    }

    fn apply_input_state(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.emu
            .send(EmuCommand::ApplyInputState(bytes))
            .map_err(|e| e.to_string())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn export_state(&self) -> Result<RuntimeStateExport, String> {
        let (tx, rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::SaveState(tx))
            .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map(|state| RuntimeStateExport {
                state_blob: state,
                preview: None,
            })
            .map_err(|e| e.to_string())
    }

    fn import_state(&mut self, state_blob: &[u8]) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::LoadState(state_blob.to_vec(), tx))
            .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }

    fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }

    fn import_mapper_save(&self, _bytes: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity> {
        None
    }

    fn with_frame_buffer(&self, f: &mut dyn FnMut(&[u8])) {
        if let Ok(result) = self.emu.wait_frame() {
            f(&result.slot_data);
        }
    }
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
        SystemSettingsChoiceId, SystemSettingsFieldId, default_input_topology_descriptor,
        default_system_definition,
    };
    use crate::load::SystemLoadOptions;
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
        let definition = default_system_definition();
        let settings = snapshot();

        let resolved = definition
            .resolve_load_request(
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
        let definition = default_system_definition();
        let mut settings = snapshot();

        definition
            .apply_settings_choice(
                &mut settings,
                &SystemSettingsFieldId(Cow::Borrowed("core.mmc3_irq_variant")),
                &SystemSettingsChoiceId(Cow::Borrowed("sharp")),
            )
            .unwrap();

        let page = definition.settings_page(&settings);
        assert_eq!(page.fields.len(), 2);
    }
}
