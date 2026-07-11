use std::rc::Rc;

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use nerust_core_traits::factory::load::{MediaObject, SystemLoadOptions};
use nerust_core_traits::factory::{CoreFactory, FactoryError};
use nerust_core_traits::identity::SystemId;
use nerust_core_traits::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError,
    audio::{AudioBackend, AudioBackendRegistry},
    factory::settings::FactorySettingsView,
    identity::SystemIdentity,
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsApplyPlan,
};
use nerust_input_traits::{
    BufferError, ControllerCollection, ControllerProfile, CreateSplitError, GuiInput,
    InputAssignments, InputPorts, InputResources, InputSplit, InputStateBuffer, InputSystemFactory,
    InputValue, SlotInfo,
};
use nerust_persistence::slots::autosave_state_slot_path;
use nerust_render_base::logical::LogicalSize;
use nerust_render_base::physical::PhysicalSize;
use nerust_render_base::{FrameBuffer, VideoRenderProfile};

use crate::session::{KeyboardShortcut, SessionHandle};
use crate::settings::factory::settings_view;

/// Minimal InputStateBuffer for testing.
#[derive(Debug, Default)]
struct TestInputBuffer([u8; 2]);

impl InputStateBuffer for TestInputBuffer {
    fn set(&mut self, _field: usize, _value: InputValue) -> Result<(), BufferError> {
        Ok(())
    }
    fn clear(&mut self) {
        self.0 = [0; 2];
    }
    fn copy_state(&mut self, other: &dyn InputStateBuffer) {
        let any: &dyn std::any::Any = other;
        if let Some(src) = any.downcast_ref::<TestInputBuffer>() {
            self.0 = src.0;
        }
    }
}

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
            video_signal: nerust_core_traits::VideoSignalKind::Ntsc,
        }
    }
    fn render_frame(&mut self, _frame_slot: &mut FrameBuffer) -> Result<(), CoreError> {
        Ok(())
    }

    fn load(&mut self, rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        self.loaded = true;
        self.paused = true;
        self.identity = Some(SystemIdentity::new(
            SystemId::new("nes"),
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

fn test_input_resources() -> (GuiInput, InputSplit) {
    let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
        Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
    let flag = Arc::new(AtomicBool::new(false));
    let gui = GuiInput {
        shared: Arc::clone(&shared),
        flag: Arc::clone(&flag),
        state: Box::<TestInputBuffer>::default(),
        write_buf: Box::<TestInputBuffer>::default(),
    };
    let split = InputSplit {
        shared: Arc::clone(&shared),
        flag: Arc::clone(&flag),
        new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
    };
    (gui, split)
}

fn build_test_core_parts() -> nerust_core_traits::factory::CoreParts {
    use nerust_core_traits::factory::CoreParts;
    let core = MockConsoleCore::new();
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
        frame_format: nerust_render_base::VideoFrameFormat::Palette,
        ntsc_packed_rgba8: None,
    };
    let (gui_input, _input_split) = test_input_resources();
    CoreParts {
        core: Box::new(core),
        gui_input,
        field_map: std::collections::HashMap::new(),
        render_profile,
        palette: Box::new([0u32; 256]),
    }
}

#[derive(Debug)]
struct MockInputFactory;
impl InputPorts for MockInputFactory {
    fn slots(&self) -> &[SlotInfo] {
        &[]
    }
    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        vec![]
    }
}
impl InputSystemFactory for MockInputFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments { slots: vec![] }
    }
    fn create_split(&self, _: &ControllerCollection) -> Result<InputResources, CreateSplitError> {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<TestInputBuffer>::default()));
        let flag = Arc::new(AtomicBool::new(false));
        Ok(InputResources {
            split: InputSplit {
                shared: Arc::clone(&shared),
                flag: Arc::clone(&flag),
                new_buffer: Box::new(|| Box::<TestInputBuffer>::default()),
            },
            field_map: std::collections::HashMap::new(),
        })
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

    fn create_core_and_adapter_with_assignments(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
        _speaker: Box<dyn AudioBackend>,
        _: &InputAssignments,
    ) -> Result<nerust_core_traits::factory::CoreParts, FactoryError> {
        Ok(build_test_core_parts())
    }
    fn probe_media(&self, _: &MediaObject) -> bool {
        true
    }
    fn system_descriptor(&self) -> nerust_core_traits::factory::descriptor::SystemDescriptor {
        nerust_core_traits::factory::descriptor::SystemDescriptor
    }
    fn settings_page(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
    ) -> nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
        nerust_core_traits::factory::descriptor::SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }
    fn apply_settings_choice(
        &self,
        _: &mut nerust_core_traits::factory::settings::FactorySettingsView,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsFieldId,
        _: &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        Ok(())
    }
    fn resolve_load_request(
        &self,
        _: &nerust_core_traits::factory::settings::FactorySettingsView,
        options: SystemLoadOptions,
    ) -> Result<nerust_core_traits::factory::load::ResolvedLoadRequest, FactoryError> {
        let bytes = options.options_bytes.clone();
        Ok(nerust_core_traits::factory::load::ResolvedLoadRequest {
            options,
            core_options_bytes: bytes,
        })
    }
    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        static MOCK_INPUT: MockInputFactory = MockInputFactory;
        &MOCK_INPUT
    }
}

fn test_session() -> SessionHandle {
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    };
    let factory: Arc<dyn CoreFactory> = Arc::new(MockFactory);
    let descriptor = factory.system_descriptor();
    let audio_registry = Arc::new(AudioBackendRegistry::new());
    // Use ephemeral settings so tests are not affected by disk state.
    SessionHandle::new_ephemeral(capabilities, factory, audio_registry)
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

fn test_view(session: &SessionHandle) -> FactorySettingsView {
    let system_id = session.factory().system_id();
    settings_view(session.settings_snapshot(), &system_id)
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
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), SystemLoadOptions::default())
        .unwrap();
    assert!(
        session
            .load_resolved(MediaObject::new(None, test_rom()), resolved)
            .is_ok()
    );
}

#[test]
fn session_rebuild_reuses_previously_resolved_load_request() {
    let mut session = test_session();
    let options = session.factory().default_load_options();
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
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
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
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
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
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
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
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
    let resolved = session
        .factory()
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(
            MediaObject::new(Some(rom_path.clone()), test_rom()),
            resolved,
        )
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
    let resolved = session2
        .factory()
        .resolve_load_request(&test_view(&session2), options)
        .unwrap();
    session2
        .load_resolved(
            MediaObject::new(Some(rom_path), test_rom_with_mapper4()),
            resolved,
        )
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
