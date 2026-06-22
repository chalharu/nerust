use crate::emu_core::EmuCore;
use crate::factory::{CoreFactory, FactoryError};
use crate::load::{MediaObject, SystemLoadOptions};
use crate::session::{KeyboardShortcut, SessionHandle};
use nerust_contract_core::ConsoleCore;
use nerust_contract_core::identity::SystemIdentity;
use nerust_contract_core::input::{InputStatePersistence, SystemInputAdapter};
use nerust_contract_core::{CoreCapabilities, CoreConfig, CoreError, GpuCommandList};
use nerust_contract_emuthread::EmuThread;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsApplyPlan, SettingsSnapshot};

use nerust_persistence::slots::autosave_state_slot_path;
use nerust_screen_video::{
    FrameBuffer, LogicalSize, PhysicalSize, PixelFormat, VideoRenderProfile,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::test_support::single_port_topology;
use nerust_contract_input::SystemId;

struct MockConsoleCore {
    loaded: bool,
    paused: bool,
    identity: Option<SystemIdentity>,
}

impl MockConsoleCore {
    fn new() -> Self {
        Self {
            loaded: false,
            paused: true,
            identity: None,
        }
    }
}

impl ConsoleCore for MockConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: Vec::new(),
            video_signal: nerust_contract_core::VideoSignalKind::Ntsc,
        }
    }
    fn render_frame(&mut self, _frame_slot: &mut FrameBuffer) -> Result<GpuCommandList, CoreError> {
        Ok(GpuCommandList {
            commands: Vec::new(),
        })
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        self.loaded = true;
        self.paused = true;
        self.identity = Some(SystemIdentity::new(
            nerust_contract_input::SystemId::new("nes"),
            rom.get(6..8).unwrap_or(&[0, 0]).to_vec(),
        ));
        Ok(())
    }
    fn unload(&mut self) {
        self.loaded = false;
    }
    fn reset(&mut self) {}
    fn paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        Ok(vec![])
    }
    fn load_state(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn identity(&self) -> Result<SystemIdentity, CoreError> {
        self.identity.clone().ok_or(CoreError::NoRomLoaded)
    }
}

fn build_test_core_and_adapter() -> (EmuCore, Box<dyn SystemInputAdapter>) {
    let src_w = 256usize;
    let src_h = 240usize;
    let pixel_format = PixelFormat::PaletteIndex {
        palette: Box::new([0u32; 256]),
    };

    let shared_fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
        src_w,
        src_h,
        pixel_format.clone(),
    )));
    if let Ok(mut guard) = shared_fb.lock() {
        guard.resize(src_w, src_h);
        guard.resize_data(src_w * src_h);
    }

    let mut disp_fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format.clone());
    disp_fb.resize(src_w, src_h);
    disp_fb.resize_data(src_w * src_h);

    let core = MockConsoleCore::new();
    let frame_ready = Arc::new(AtomicBool::new(false));
    let palette = Box::new([0u32; 256]);
    let emu = EmuThread::spawn(
        Box::new(core),
        Arc::clone(&shared_fb),
        Arc::clone(&frame_ready),
        palette,
    );

    let render_profile = VideoRenderProfile {
        source_logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        logical_size: LogicalSize {
            width: 256,
            height: 240,
        },
        physical_size: PhysicalSize {
            width: 256.0,
            height: 240.0,
        },
        frame_format: nerust_screen_video::VideoFrameFormat::Palette,
        ntsc_packed_rgba8: None,
    };

    let emu_core = EmuCore::new(emu, render_profile, shared_fb, disp_fb, frame_ready);
    let adapter = Box::new(MockAdapter);
    (emu_core, adapter)
}

struct MockAdapter;
impl SystemInputAdapter for MockAdapter {
    fn apply_event(&mut self, _: nerust_contract_input::DigitalInputEvent) {}
    fn clear(&mut self) {}
    fn decode_persisted_input(
        &self,
        _: &str,
        _: &str,
        _: bool,
    ) -> Option<nerust_contract_input::DigitalInputEvent> {
        None
    }
}
impl InputStatePersistence for MockAdapter {
    fn sync_from_runtime_state(
        &mut self,
        _: &[u8],
    ) -> Result<(), nerust_contract_core::input::InputError> {
        Ok(())
    }
    fn runtime_state_bytes(&self) -> Result<Vec<u8>, nerust_contract_core::input::InputError> {
        Ok(Vec::new())
    }
}

struct MockFactory;
impl CoreFactory for MockFactory {
    fn system_id(&self) -> SystemId {
        SystemId::new("nes")
    }

    fn display_name(&self) -> &'static str {
        "NES (test)"
    }

    fn create_core_and_adapter(
        &self,
        _: &SettingsSnapshot,
    ) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError> {
        Ok(build_test_core_and_adapter())
    }
    fn probe_media(&self, _: &MediaObject) -> bool {
        true
    }
    fn system_descriptor(&self) -> crate::descriptor::SystemDescriptor {
        crate::descriptor::SystemDescriptor {
            input_topology: single_port_topology(),
        }
    }
    fn settings_page(&self, _: &SettingsSnapshot) -> crate::descriptor::SystemSettingsPageModel {
        crate::descriptor::SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }
    fn apply_settings_choice(
        &self,
        _: &mut SettingsSnapshot,
        _: &crate::descriptor::SystemSettingsFieldId,
        _: &crate::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        Ok(())
    }
    fn resolve_load_request(
        &self,
        _: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<crate::load::ResolvedLoadRequest, FactoryError> {
        let bytes = options.options_bytes.clone();
        Ok(crate::load::ResolvedLoadRequest {
            options,
            core_options_bytes: bytes,
        })
    }
    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }
}

fn test_session() -> SessionHandle {
    let identity = HostBackendIdentity::gtk_opengl();
    let (core, adapter) = build_test_core_and_adapter();
    let factory: Arc<dyn CoreFactory> = Arc::new(MockFactory);
    let descriptor = factory.system_descriptor();
    SessionHandle::new_with_core(identity, descriptor, factory, core, adapter)
}

fn test_rom() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
}

fn test_rom_with_mapper4() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0x40, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
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
fn shortcut_key_returns_shortcut_action_without_controller_event() {
    let mut session = test_session();
    assert_eq!(
        session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true),
        Some(KeyboardShortcut::Session(
            nerust_gui_settings::input::ShortcutAction::TogglePause
        )),
    );
    assert_eq!(
        session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true),
        None
    );
}

#[test]
fn system_load_options_flow_into_session_load() {
    let mut session = test_session();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), SystemLoadOptions::default())
        .unwrap();
    assert!(
        session
            .load_resolved(MediaObject::new(None, test_rom()))
            .is_ok()
    );
}

#[test]
fn session_rebuild_reuses_previously_resolved_load_request() {
    let mut session = test_session();
    let options = session.factory().default_load_options();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom()))
        .unwrap();
    assert!(session.loaded());

    let mut next = session.settings_snapshot().clone();
    next.local.audio.latency_ms = 90;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    assert!(session.loaded());
}

#[test]
fn rebuild_preserves_restored_runtime_state_without_reloading_mapper_save() {
    let temp_dir = unique_temp_dir("rebuild");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session.factory().default_load_options();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()))
        .unwrap();

    let mapper_save_path = session
        .persistence
        .mapper_save_path()
        .expect("load should configure mapper_save_path")
        .clone();
    fs::write(&mapper_save_path, [9, 8, 7, 6]).expect("mapper save should write");

    let mut next = session.settings_snapshot().clone();
    next.local.audio.latency_ms = 90;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    assert!(mapper_save_path.exists());
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_round_trips_without_visible_slot() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-state");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session.factory().default_load_options();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()))
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let autosave_path = autosave_state_slot_path(
        session
            .persistence
            .states_dir()
            .expect("load should configure states_dir"),
    );
    assert!(autosave_path.is_file());
    assert!(session.slots().is_empty());
    assert_eq!(session.active_slot_id(), None);

    assert!(session.load_hidden_lifecycle_state());
    assert_eq!(session.slots().len(), 0);
    assert_eq!(session.active_slot_id(), None);

    drop(session);
    assert!(autosave_path.exists());
    fs::remove_file(&autosave_path).ok();
    assert!(!autosave_path.exists());
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_is_deleted_after_import_failure() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-import");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session.factory().default_load_options();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()))
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let autosave_path = autosave_state_slot_path(
        session
            .persistence
            .states_dir()
            .expect("load should configure states_dir"),
    );
    assert!(autosave_path.is_file());

    fs::write(&autosave_path, [0xFF, 0xFF, 0xFF]).expect("corrupt state");
    assert!(!session.load_hidden_lifecycle_state());
    assert!(!autosave_path.exists());
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_is_deleted_after_identity_mismatch() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-identity");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session.factory().default_load_options();
    let _resolved = session
        .factory()
        .resolve_load_request(session.settings_snapshot(), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path.clone()), test_rom()))
        .unwrap();
    assert!(session.save_hidden_lifecycle_state());

    let autosave_path = autosave_state_slot_path(
        session
            .persistence
            .states_dir()
            .expect("load should configure states_dir"),
    );
    assert!(autosave_path.is_file());
    drop(session);

    let mut session2 = test_session();
    let options = session2.factory().default_load_options();
    let _resolved = session2
        .factory()
        .resolve_load_request(session2.settings_snapshot(), options)
        .unwrap();
    session2
        .load_resolved(MediaObject::new(Some(rom_path), test_rom_with_mapper4()))
        .unwrap();
    assert!(!session2.load_hidden_lifecycle_state());
    assert!(!autosave_path.exists());
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn set_fullscreen_default_updates_snapshot_and_plan() {
    let mut session = test_session();
    session.handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::KeyZ, true);
    let plan = session
        .set_fullscreen_default(true)
        .expect("set_fullscreen_default should succeed");
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
    let second = session
        .set_fullscreen_default(true)
        .expect("second set_fullscreen_default should succeed");
    assert_eq!(second, SettingsApplyPlan::default());
}
