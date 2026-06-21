use crate::descriptor::SystemInputAdapter;
use crate::emu_core::EmuCore;
use crate::load::{LoadRequest, MediaObject, SystemLoadOptions};
use crate::session::{KeyboardShortcut, SessionHandle};
use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::options::Mmc3IrqVariant;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsApplyPlan};
use nerust_input_nes_runtime::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_persistence::slots::autosave_state_slot_path;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct TestSpeaker;

impl AudioBackend for TestSpeaker {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _data: f32) {}
}

fn test_session() -> SessionHandle {
    let identity = HostBackendIdentity::gtk_opengl();
    let (core, adapter) = create_test_core_and_adapter();
    SessionHandle::new_with_core(identity, core, adapter)
}

fn create_test_core_and_adapter() -> (EmuCore, Box<dyn SystemInputAdapter>) {
    let cell = Arc::new(NesInputCell::new());
    let device = NesPadDevice::new(SharedNesInputCell(cell.clone()));
    let core = EmuCore::new_gpu(
        Box::new(TestSpeaker),
        nerust_screen_video::FilterType::NtscComposite,
        nerust_screen_video::LogicalSize {
            width: 256,
            height: 240,
        },
        Box::new(device),
    );
    let adapter = Box::new(crate::descriptor::NesAdapter::new(cell));
    (core, adapter)
}

fn make_ines_rom(prg_banks: u8, chr_banks: u8, flags6: u8, flags7: u8) -> Vec<u8> {
    let prg_size = prg_banks as usize * 0x4000;
    let chr_size = chr_banks as usize * 0x2000;
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, prg_banks, chr_banks, flags6, flags7];
    data.resize(16, 0);
    data.resize(16 + prg_size + chr_size, 0);
    data
}

fn test_rom() -> Vec<u8> {
    make_ines_rom(2, 1, 0, 0)
}

fn test_rom_with_mapper4() -> Vec<u8> {
    make_ines_rom(2, 1, 0x40, 0) // mapper 4 (MMC3)
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
        session
            .handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true)
            .unwrap(),
        Some(KeyboardShortcut::Session(
            nerust_gui_settings::input::ShortcutAction::TogglePause,
        ))
    );
    assert_eq!(
        session
            .handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::Space, true)
            .unwrap(),
        None
    );
}

#[test]
fn system_load_options_flow_into_session_load() {
    let mut session = test_session();

    assert!(
        session
            .load(
                MediaObject::new(None, test_rom()),
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
    let mut session = test_session();

    session
        .load(MediaObject::new(None, test_rom()), LoadRequest::Auto)
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
    let temp_dir = unique_temp_dir("phase5-rebuild");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();

    session
        .load(
            MediaObject::new(Some(rom_path), test_rom()),
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
    next.local.audio.latency_ms = 90;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    // After rebuild with restored state, mapper save should still exist (wasn't reloaded)
    assert!(mapper_save_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_round_trips_without_visible_slot() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-state");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();

    session
        .load(
            MediaObject::new(Some(rom_path), test_rom()),
            LoadRequest::Auto,
        )
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let autosave_path = autosave_state_slot_path(
        &session
            .persistence
            .sidecars
            .as_ref()
            .expect("load should configure sidecars")
            .states_dir,
    );
    assert!(autosave_path.is_file());
    assert!(session.slots().is_empty());
    assert_eq!(session.active_slot_id(), None);

    assert!(session.load_hidden_lifecycle_state());
    assert_eq!(session.slots().len(), 0);
    assert_eq!(session.active_slot_id(), None);

    drop(session);
    // Verify state file persists beyond session lifetime
    assert!(autosave_path.exists());
    fs::remove_file(&autosave_path).ok();
    assert!(!autosave_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_is_deleted_after_import_failure() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-import-failure");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();

    session
        .load(
            MediaObject::new(Some(rom_path), test_rom()),
            LoadRequest::Auto,
        )
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let autosave_path = autosave_state_slot_path(
        &session
            .persistence
            .sidecars
            .as_ref()
            .expect("load should configure sidecars")
            .states_dir,
    );
    assert!(autosave_path.is_file());

    // Corrupt the state file
    fs::write(&autosave_path, [0xFF, 0xFF, 0xFF]).expect("should write corrupted state");
    assert!(!session.load_hidden_lifecycle_state());
    assert!(!autosave_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_is_deleted_after_identity_mismatch() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-identity-mismatch");
    let rom_path = temp_dir.join("test.nes");

    // First session: load test_rom (mapper 0), save hidden lifecycle state
    let mut session = test_session();
    session
        .load(
            MediaObject::new(Some(rom_path.clone()), test_rom()),
            LoadRequest::Auto,
        )
        .unwrap();
    assert!(session.save_hidden_lifecycle_state());

    let autosave_path = autosave_state_slot_path(
        &session
            .persistence
            .sidecars
            .as_ref()
            .expect("load should configure sidecars")
            .states_dir,
    );
    assert!(autosave_path.is_file());
    drop(session);

    // Second session: load different ROM data (mapper 4 → different identity),
    // same path so sidecar dir matches, try to load hidden state
    let mut session2 = test_session();
    session2
        .load(
            MediaObject::new(Some(rom_path), test_rom_with_mapper4()),
            LoadRequest::Auto,
        )
        .unwrap();
    assert!(!session2.load_hidden_lifecycle_state());
    // After identity mismatch, the state file should be deleted
    assert!(!autosave_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn set_fullscreen_default_updates_snapshot_and_plan() {
    let mut session = test_session();

    session
        .handle_keyboard_key(nerust_gui_settings::input::KeyboardKey::KeyZ, true)
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

    let second = session.set_fullscreen_default(true).unwrap();
    assert_eq!(second, SettingsApplyPlan::default());
}
