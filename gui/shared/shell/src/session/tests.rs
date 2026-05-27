use crate::descriptor::{
    RuntimeHostServices, SystemDefinition, SystemDescriptor, SystemInputAdapter, SystemRuntime,
    SystemRuntimeSnapshot, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
    default_input_topology_descriptor, default_system_definition,
};
use crate::load::{LoadRequest, MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use crate::session::{KeyboardShortcut, SessionHandle};
use crate::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use nerust_console::ConsoleMetrics;
use nerust_console::state::RuntimeStateExport;
use nerust_contract_mirror::MirrorMode;
use nerust_contract_options::Mmc3IrqVariant;
use nerust_contract_persistence::CanonicalMediaIdentity;
use nerust_contract_rom::{RomFormat, RomIdentity};
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsApplyPlan, SettingsSnapshot};
use nerust_gui_session::core::SessionCore;
use nerust_input_nes::codec::decode_input_state;
use nerust_input_nes::frame::{Buttons, NesInputFrame};
use nerust_input_schema::{DigitalInputEvent, SystemId};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_traits::{MixerInput, Sound};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct TestSpeaker;

impl Sound for TestSpeaker {
    fn start(&mut self) {}

    fn pause(&mut self) {}
}

impl MixerInput for TestSpeaker {
    fn push(&mut self, _data: f32) {}
}

struct TestRuntime(SessionCore);

impl SystemRuntime for TestRuntime {
    fn snapshot(&self) -> SystemRuntimeSnapshot {
        SystemRuntimeSnapshot {
            metrics: self.0.metrics(),
            video_frame: Some(self.0.video_frame_handle()),
            video_profile: Some(self.0.video_render_profile()),
        }
    }

    fn load(&mut self, media: &MediaObject, request: &ResolvedLoadRequest) -> Result<(), String> {
        self.0
            .load_rom(media.bytes.as_ref().to_vec(), request.core_options)
            .map_err(|error| error.to_string())
    }

    fn unload(&mut self) -> Result<bool, String> {
        self.0
            .unload_rom()
            .map(|_| true)
            .map_err(|error| error.to_string())
    }

    fn reset(&self) -> Result<(), String> {
        self.0.reset().map_err(|error| error.to_string())
    }

    fn pause(&mut self) {
        self.0.pause();
    }

    fn resume(&mut self) {
        self.0.resume();
    }

    fn apply_input_state(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.0.apply_input_state(bytes);
        Ok(())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        self.0
            .current_input_state()
            .map_err(|error| error.to_string())
    }

    fn export_state(&self) -> Result<RuntimeStateExport, String> {
        self.0.export_state().map_err(|error| error.to_string())
    }

    fn import_state(&mut self, state_blob: &[u8]) -> Result<(), String> {
        self.0
            .import_state(state_blob.to_vec())
            .map_err(|error| error.to_string())
    }

    fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, String> {
        self.0
            .export_mapper_save()
            .map_err(|error| error.to_string())
    }

    fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.0
            .import_mapper_save(bytes)
            .map_err(|error| error.to_string())
    }

    fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity> {
        self.0.canonical_media_identity().ok()
    }
}

fn test_session() -> SessionHandle {
    SessionHandle::from_runtime(
        HostBackendIdentity::gtk_opengl(),
        Box::new(TestRuntime(SessionCore::from_console(
            nerust_console::Console::new(
                TestSpeaker,
                ScreenBuffer::new_nes_gpu_default(),
                nerust_input_nes_runtime::standard_controller_runtime(),
            ),
        ))),
        default_system_definition(),
    )
}

#[derive(Clone, Default)]
struct RecordedLoads(Arc<Mutex<Vec<ResolvedLoadRequest>>>);

struct MockDefinition {
    loads: RecordedLoads,
}

struct MockRuntime {
    loads: RecordedLoads,
    loaded: bool,
    paused: bool,
}

#[derive(Default)]
struct MockInputAdapter;

impl SystemInputAdapter for MockInputAdapter {
    fn digital_event_from_persisted(
        &self,
        _attachment: &str,
        _control: &str,
        _pressed: bool,
    ) -> Option<DigitalInputEvent> {
        None
    }

    fn apply_event(&mut self, _event: DigitalInputEvent) {}

    fn clear(&mut self) {}

    fn sync_from_runtime_state(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

impl SystemRuntime for MockRuntime {
    fn snapshot(&self) -> SystemRuntimeSnapshot {
        SystemRuntimeSnapshot {
            metrics: ConsoleMetrics {
                loaded: self.loaded,
                paused: self.paused,
                ..ConsoleMetrics::default()
            },
            video_frame: None,
            video_profile: None,
        }
    }

    fn load(&mut self, _media: &MediaObject, request: &ResolvedLoadRequest) -> Result<(), String> {
        self.loads.0.lock().unwrap().push(*request);
        self.loaded = true;
        self.paused = true;
        Ok(())
    }

    fn unload(&mut self) -> Result<bool, String> {
        self.loaded = false;
        Ok(true)
    }

    fn reset(&self) -> Result<(), String> {
        Ok(())
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    fn resume(&mut self) {
        self.paused = false;
    }

    fn apply_input_state(&mut self, _bytes: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn export_state(&self) -> Result<RuntimeStateExport, String> {
        Ok(RuntimeStateExport {
            state_blob: vec![1, 2, 3],
            preview: None,
        })
    }

    fn import_state(&mut self, _state_blob: &[u8]) -> Result<(), String> {
        Ok(())
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
}

impl SystemDefinition for MockDefinition {
    fn descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            system_id: SystemId::Nes,
            input_topology: default_input_topology_descriptor(),
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
        let resolved = if options.mmc3_irq_variant.is_some() {
            options
        } else if settings.local.audio.muted {
            SystemLoadOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            }
        } else {
            SystemLoadOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            }
        };
        Ok(ResolvedLoadRequest {
            system_id: SystemId::Nes,
            options: resolved,
            core_options: resolved.into_core_options(),
        })
    }

    fn settings_page(&self, _settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }

    fn apply_settings_choice(
        &self,
        _settings: &mut SettingsSnapshot,
        _field: &SystemSettingsFieldId,
        _choice: &SystemSettingsChoiceId,
    ) -> Result<(), String> {
        Ok(())
    }

    fn create_input_adapter(&self, _settings: &SettingsSnapshot) -> Box<dyn SystemInputAdapter> {
        Box::new(MockInputAdapter)
    }

    fn create_runtime(
        &self,
        _host: &RuntimeHostServices,
        _settings: &SettingsSnapshot,
    ) -> Result<Box<dyn SystemRuntime>, String> {
        Ok(Box::new(MockRuntime {
            loads: self.loads.clone(),
            loaded: false,
            paused: true,
        }))
    }
}

#[derive(Debug, Default)]
struct ImportCounts {
    state_imports: usize,
    mapper_save_imports: usize,
}

#[derive(Clone, Default)]
struct ImportCounters(Arc<Mutex<ImportCounts>>);

struct PersistenceDefinition {
    imports: ImportCounters,
    identity: CanonicalMediaIdentity,
}

struct PersistenceRuntime {
    imports: ImportCounters,
    identity: CanonicalMediaIdentity,
    loaded: bool,
    paused: bool,
}

impl SystemRuntime for PersistenceRuntime {
    fn snapshot(&self) -> SystemRuntimeSnapshot {
        SystemRuntimeSnapshot {
            metrics: ConsoleMetrics {
                loaded: self.loaded,
                paused: self.paused,
                ..ConsoleMetrics::default()
            },
            video_frame: None,
            video_profile: None,
        }
    }

    fn load(&mut self, _media: &MediaObject, _request: &ResolvedLoadRequest) -> Result<(), String> {
        self.loaded = true;
        self.paused = true;
        Ok(())
    }

    fn unload(&mut self) -> Result<bool, String> {
        self.loaded = false;
        Ok(true)
    }

    fn reset(&self) -> Result<(), String> {
        Ok(())
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    fn resume(&mut self) {
        self.paused = false;
    }

    fn apply_input_state(&mut self, _bytes: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    fn export_state(&self) -> Result<RuntimeStateExport, String> {
        Ok(RuntimeStateExport {
            state_blob: vec![1, 2, 3],
            preview: None,
        })
    }

    fn import_state(&mut self, _state_blob: &[u8]) -> Result<(), String> {
        self.imports.0.lock().unwrap().state_imports += 1;
        Ok(())
    }

    fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }

    fn import_mapper_save(&self, _bytes: Vec<u8>) -> Result<(), String> {
        self.imports.0.lock().unwrap().mapper_save_imports += 1;
        Ok(())
    }

    fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity> {
        Some(self.identity)
    }
}

impl SystemDefinition for PersistenceDefinition {
    fn descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            system_id: SystemId::Nes,
            input_topology: default_input_topology_descriptor(),
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
        _settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, String> {
        Ok(ResolvedLoadRequest {
            system_id: SystemId::Nes,
            options,
            core_options: options.into_core_options(),
        })
    }

    fn settings_page(&self, _settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }

    fn apply_settings_choice(
        &self,
        _settings: &mut SettingsSnapshot,
        _field: &SystemSettingsFieldId,
        _choice: &SystemSettingsChoiceId,
    ) -> Result<(), String> {
        Ok(())
    }

    fn create_input_adapter(&self, _settings: &SettingsSnapshot) -> Box<dyn SystemInputAdapter> {
        Box::new(MockInputAdapter)
    }

    fn create_runtime(
        &self,
        _host: &RuntimeHostServices,
        _settings: &SettingsSnapshot,
    ) -> Result<Box<dyn SystemRuntime>, String> {
        Ok(Box::new(PersistenceRuntime {
            imports: self.imports.clone(),
            identity: self.identity,
            loaded: false,
            paused: true,
        }))
    }
}

fn test_rom_identity() -> RomIdentity {
    RomIdentity {
        format: RomFormat::INes,
        mapper_type: 4,
        sub_mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: true,
        trainer_len: 0,
        prg_rom_len: 0x8000,
        chr_rom_len: 0x2000,
        prg_ram_len: 0,
        save_prg_ram_len: 0x2000,
        chr_ram_len: 0,
        save_chr_ram_len: 0,
        prg_rom_crc64: 1,
        chr_rom_crc64: 2,
        trainer_crc64: 3,
    }
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("nerust-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).expect("temp dir should create");
    path
}

#[test]
fn session_flushes_keyboard_input_into_controller_state() {
    let mut session = test_session();

    assert_eq!(
        session
            .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::KeyZ, true)
            .unwrap(),
        None
    );

    let frame = decode_input_state(
        &session
            .runtime
            .current_input_state()
            .expect("input state should export"),
    )
    .expect("input state should decode");
    assert_eq!(
        frame,
        NesInputFrame {
            player_one: Buttons::A,
            player_two: Buttons::empty(),
            microphone: false,
        }
    );
}

fn build_snes_lorom() -> Vec<u8> {
    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;
    let mut rom = vec![0; 0x10000];
    rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST GUI SNES ROM    ");
    rom[0x7FD5] = 0x30;
    rom[0x7FD7] = 0x08;
    rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2].copy_from_slice(&0x8000u16.to_le_bytes());
    rom
}

#[test]
fn shortcut_key_returns_shortcut_action_without_controller_event() {
    let mut session = test_session();

    assert_eq!(
        session
            .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::Space, true)
            .unwrap(),
        Some(KeyboardShortcut::Session(
            nerust_contract_settings::input::ShortcutAction::TogglePause,
        ))
    );
    assert_eq!(
        session
            .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::Space, true)
            .unwrap(),
        None
    );
}

#[test]
fn auto_load_switches_to_snes_runtime_for_sfc_media() {
    let mut session = test_session();

    session
        .load(
            MediaObject::new(Some(PathBuf::from("game.sfc")), build_snes_lorom()),
            LoadRequest::Auto,
        )
        .unwrap();

    let snapshot = session.snapshot();
    assert_eq!(snapshot.system_id, Some(SystemId::Snes));
    assert!(snapshot.metrics.loaded);
    assert!(snapshot.metrics.paused);
    assert_eq!(snapshot.video_frame.unwrap().bytes().len(), 256 * 224 * 4);
    assert_eq!(session.input_topology_descriptor().system, SystemId::Snes);
}

#[test]
fn snes_session_flushes_keyboard_input_into_standard_pad_state() {
    let mut session = test_session();

    session
        .load(
            MediaObject::new(Some(PathBuf::from("game.sfc")), build_snes_lorom()),
            LoadRequest::Auto,
        )
        .unwrap();

    assert_eq!(
        session
            .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::KeyZ, true)
            .unwrap(),
        None
    );
    assert_eq!(
        session
            .runtime
            .current_input_state()
            .expect("SNES input state should export"),
        vec![0x00, 0x80]
    );

    assert_eq!(
        session
            .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::KeyZ, false)
            .unwrap(),
        None
    );
    assert_eq!(
        session
            .runtime
            .current_input_state()
            .expect("SNES input state should export"),
        vec![0x00, 0x00]
    );
}

#[test]
fn system_load_options_flow_into_session_load() {
    let mut session = test_session();
    let mut rom = vec![
        0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    rom.resize(16 + 0x8000 + 0x2000, 0);

    assert!(
        session
            .load(
                MediaObject::new(None, rom),
                LoadRequest::Explicit {
                    system_id: nerust_input_schema::SystemId::Nes,
                    options: SystemLoadOptions {
                        mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
                    },
                },
            )
            .is_ok()
    );
}

#[test]
fn session_rebuild_reuses_previously_resolved_load_request() {
    let loads = RecordedLoads::default();
    let definition: Box<dyn SystemDefinition> = Box::new(MockDefinition {
        loads: loads.clone(),
    });
    let runtime = definition
        .create_runtime(
            &RuntimeHostServices {
                host_backend: HostBackendIdentity::gtk_opengl(),
            },
            &SettingsSnapshot {
                shared: default_shared_settings(),
                local: default_local_settings(),
                app_state: default_app_state(),
            },
        )
        .unwrap();
    let mut session =
        SessionHandle::from_runtime(HostBackendIdentity::gtk_opengl(), runtime, definition);

    session
        .load(MediaObject::new(None, vec![0; 16]), LoadRequest::Auto)
        .unwrap();
    let mut next = session.settings_snapshot().clone();
    next.local.audio.muted = true;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    let loads = loads.0.lock().unwrap();
    assert_eq!(loads.len(), 2);
    assert_eq!(
        loads[0].options.mmc3_irq_variant,
        Some(Mmc3IrqVariant::Sharp)
    );
    assert_eq!(
        loads[1].options.mmc3_irq_variant,
        Some(Mmc3IrqVariant::Sharp)
    );
}

#[test]
fn rebuild_preserves_restored_runtime_state_without_reloading_mapper_save() {
    let temp_dir = unique_temp_dir("phase5-rebuild");
    let rom_path = temp_dir.join("test.nes");
    let imports = ImportCounters::default();
    let identity = CanonicalMediaIdentity::rom(test_rom_identity());
    let definition: Box<dyn SystemDefinition> = Box::new(PersistenceDefinition {
        imports: imports.clone(),
        identity,
    });
    let runtime = definition
        .create_runtime(
            &RuntimeHostServices {
                host_backend: HostBackendIdentity::gtk_opengl(),
            },
            &SettingsSnapshot {
                shared: default_shared_settings(),
                local: default_local_settings(),
                app_state: default_app_state(),
            },
        )
        .unwrap();
    let mut session =
        SessionHandle::from_runtime(HostBackendIdentity::gtk_opengl(), runtime, definition);

    session
        .load(
            MediaObject::new(Some(rom_path), vec![0; 16]),
            LoadRequest::Auto,
        )
        .unwrap();

    let mapper_save_path = session
        .persistence
        .sidecars
        .as_ref()
        .expect("load should configure sidecars")
        .mapper_save_path
        .clone();
    fs::write(&mapper_save_path, [9, 8, 7, 6]).expect("mapper save should write");

    let mut next = session.settings_snapshot().clone();
    next.local.audio.muted = true;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    let counts = imports.0.lock().unwrap();
    assert_eq!(counts.state_imports, 1);
    assert_eq!(counts.mapper_save_imports, 0);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn set_fullscreen_default_updates_snapshot_and_plan() {
    let mut session = test_session();

    session
        .handle_keyboard_key(nerust_contract_settings::input::KeyboardKey::KeyZ, true)
        .unwrap();

    let plan = session.set_fullscreen_default(true).unwrap();

    assert_eq!(
        plan,
        SettingsApplyPlan {
            window_settings_changed: true,
            fullscreen_default_changed: true,
            ..SettingsApplyPlan::default()
        }
    );
    assert!(
        session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default
    );
    let frame = decode_input_state(
        &session
            .runtime
            .current_input_state()
            .expect("input state should export"),
    )
    .expect("input state should decode");
    assert_eq!(
        frame,
        NesInputFrame {
            player_one: Buttons::A,
            player_two: Buttons::empty(),
            microphone: false,
        }
    );

    let second = session.set_fullscreen_default(true).unwrap();
    assert_eq!(second, SettingsApplyPlan::default());
}
