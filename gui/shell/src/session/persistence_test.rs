use std::fs;

use nerust_core_traits::factory::load::MediaObject;
use nerust_persistence::slots::autosave_state_slot_path;

use crate::session::test_util::*;
use crate::test_helpers::*;

#[test]
fn rebuild_preserves_restored_runtime_state_without_reloading_mapper_save() {
    let temp_dir = unique_temp_dir("rebuild");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
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
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
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
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
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
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
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
    let options = session2
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session2
        .factory()
        .expect("no active system")
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
