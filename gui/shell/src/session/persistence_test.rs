use std::fs;

use nerust_core_traits::factory::load::MediaObject;
use nerust_persistence::slots::autosave_state_slot_path;

use crate::emu_core::CorePersistenceError;
use crate::session::commands::{SessionCommand, SessionCommandOutcome, SlotOpFailure};
use crate::session::persistence::FailingSlotBackend;
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

fn load_test_rom(session: &mut crate::session::SessionHandle, rom_path: std::path::PathBuf) {
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
        .resolve_load_request(&test_view(session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
        .unwrap();
}

#[test]
fn slot_commands_report_empty_missing_and_clean_round_trip() {
    let temp_dir = unique_temp_dir("slot-failures");
    let mut session = test_session();
    load_test_rom(&mut session, temp_dir.join("test.nes"));

    // Nothing saved yet: Empty, not a generic failure.
    assert_eq!(
        session.run_command(SessionCommand::LoadActiveSlot).unwrap(),
        SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: Some(SlotOpFailure::Empty),
        }
    );
    // A selected but absent slot file: Missing.
    assert!(
        session
            .run_command(SessionCommand::SelectActiveSlot(7))
            .unwrap()
            .executed
    );
    assert_eq!(
        session.run_command(SessionCommand::LoadActiveSlot).unwrap(),
        SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: Some(SlotOpFailure::Missing),
        }
    );
    // Save then load round-trips with no failure attached.
    let saved = session
        .run_command(SessionCommand::SaveActiveSlotOrNew)
        .unwrap();
    assert!(saved.executed);
    assert_eq!(saved.slot_failure, None);
    // The previously selected (missing) slot 7 is reused for the save.
    assert_eq!(session.active_slot_id(), Some(7));
    let loaded = session.run_command(SessionCommand::LoadActiveSlot).unwrap();
    assert!(loaded.executed);
    assert_eq!(loaded.slot_failure, None);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn failing_slot_backend_reports_storage() {
    let temp_dir = unique_temp_dir("slot-backend-failure");
    let mut session = test_session();
    session.set_persistence_backends(
        Box::new(FailingSlotBackend),
        Box::new(FailingSlotBackend),
        Box::new(FailingSlotBackend),
    );
    load_test_rom(&mut session, temp_dir.join("test.nes"));

    assert_eq!(
        session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap()
            .slot_failure,
        Some(SlotOpFailure::Storage)
    );
    assert!(
        session
            .run_command(SessionCommand::SelectActiveSlot(1))
            .unwrap()
            .executed
    );
    assert_eq!(
        session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap()
            .slot_failure,
        Some(SlotOpFailure::Storage)
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn core_persistence_errors_classify_for_slot_messages() {
    assert_eq!(
        CorePersistenceError::WorkerUnavailable.slot_failure(),
        SlotOpFailure::Unavailable
    );
    assert_eq!(
        CorePersistenceError::NoReply.slot_failure(),
        SlotOpFailure::Unavailable
    );
    assert_eq!(
        CorePersistenceError::Core("ROM identity mismatch".into()).slot_failure(),
        SlotOpFailure::Incompatible
    );
    assert_eq!(
        CorePersistenceError::Core("unsupported schema version: 9".into()).slot_failure(),
        SlotOpFailure::Incompatible
    );
    assert_eq!(
        CorePersistenceError::Core("decode error: truncated".into()).slot_failure(),
        SlotOpFailure::Corrupt
    );
    assert_eq!(
        CorePersistenceError::Core("system: bus clock ahead of system tick".into()).slot_failure(),
        SlotOpFailure::Corrupt
    );
}
