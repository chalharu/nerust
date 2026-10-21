use std::fs;

use nerust_persistence::slots::autosave_state_slot_path;
use nerust_system_traits::factory::load::MediaObject;

use crate::emu_core::CorePersistenceError;
use crate::session::commands::{SessionCommand, SessionCommandOutcome};
use crate::session::error::SlotOpFailure;
use crate::session::persistence::{FailingSlotBackend, PersistenceBackends, StateExporter};
use crate::session::persistence_setup::{MapperRestore, PersistenceSetup};
use crate::session::test_util::*;
use crate::test_helpers::*;

#[test]
fn rebuild_preserves_restored_runtime_state_without_reloading_mapper_save() {
    let temp_dir = unique_temp_dir("rebuild");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .active_factory_cloned()
        .expect("no active system")
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
        .unwrap();

    let (_, mapper_save_path) = session
        .sidecars()
        .expect("load should configure mapper_save_path");
    fs::write(mapper_save_path.native_path().unwrap(), [9, 8, 7, 6])
        .expect("mapper save should write");

    let mut next = session.settings_snapshot().clone();
    next.shared.systems.insert(
        Box::new(DummySystemId),
        Box::new(nerust_nes_settings::NesSettings::default()),
    );
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.visual_changed);
    assert!(mapper_save_path.native_path().unwrap().exists());
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn hidden_lifecycle_state_round_trips_without_visible_slot() {
    let temp_dir = unique_temp_dir("hidden-lifecycle-state");
    let rom_path = temp_dir.join("test.nes");

    let mut session = test_session();
    let options = session
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .active_factory_cloned()
        .expect("no active system")
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let (states_dir, _) = session
        .sidecars()
        .expect("load should configure states_dir");
    let autosave_path = autosave_state_slot_path(states_dir.native_path().unwrap());
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
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .active_factory_cloned()
        .expect("no active system")
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
        .unwrap();

    assert!(session.save_hidden_lifecycle_state());
    let (states_dir, _) = session
        .sidecars()
        .expect("load should configure states_dir");
    let autosave_path = autosave_state_slot_path(states_dir.native_path().unwrap());
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
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .active_factory_cloned()
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

    let (states_dir, _) = session
        .sidecars()
        .expect("load should configure states_dir");
    let autosave_path = autosave_state_slot_path(states_dir.native_path().unwrap());
    assert!(autosave_path.is_file());
    drop(session);

    let mut session2 = test_session();
    let options = session2
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session2
        .active_factory_cloned()
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
        .active_factory_cloned()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .active_factory_cloned()
        .expect("no active system")
        .resolve_load_request(&test_view(session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
        .unwrap();
}

#[test]
fn mapper_flush_failure_vetoes_rom_replacement() {
    use crate::emu_core::{EmuCore, EmuThreadPort};
    use nerust_render_traits::{FrameBuffer, PixelFormat};
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    #[derive(Debug)]
    struct FailedExport(Arc<Mutex<FrameBuffer>>);
    impl EmuThreadPort for FailedExport {
        fn send(
            &self,
            command: nerust_core_traits::EmuCommand,
        ) -> Result<(), std::sync::mpsc::TrySendError<nerust_core_traits::EmuCommand>> {
            match command {
                nerust_core_traits::EmuCommand::MapperSave { reply } => {
                    reply
                        .send(Err(nerust_core_traits::CoreError::InvalidCoreOptions))
                        .unwrap();
                    Ok(())
                }
                other => Err(std::sync::mpsc::TrySendError::Disconnected(other)),
            }
        }
        fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
            &self.0
        }
        fn frame_count(&self) -> u64 {
            0
        }
        fn fps(&self) -> f32 {
            0.0
        }
    }
    let root = unique_temp_dir("flush-veto");
    let mut session = test_session();
    load_test_rom(&mut session, root.join("original.nes"));
    let original = session
        .media
        .loaded
        .as_ref()
        .unwrap()
        .media
        .native_path()
        .unwrap()
        .to_path_buf();
    let sidecars = session.sidecars();
    let frame = Arc::new(Mutex::new(FrameBuffer::with_capacity(
        0,
        0,
        PixelFormat::Rgba,
    )));
    let parts = build_test_core_parts();
    session.emu.runtime.as_mut().unwrap().core = Box::new(EmuCore::new(
        Arc::new(FailedExport(frame.clone())),
        parts.render_profile,
        frame,
        FrameBuffer::with_capacity(0, 0, PixelFormat::Rgba),
        Arc::new(AtomicBool::new(false)),
    ));
    let factory = session.active_factory_cloned().unwrap();
    let resolved = factory
        .resolve_load_request(&test_view(&session), factory.default_load_options())
        .unwrap();
    assert!(matches!(
        session.load_resolved(
            MediaObject::new(Some(root.join("replacement.nes")), test_rom()),
            resolved,
        ),
        Err(crate::session::SessionError::MapperFlush(_))
    ));
    assert_eq!(
        session.media.loaded.as_ref().unwrap().media.native_path(),
        Some(original.as_path())
    );
    assert_eq!(session.sidecars(), sidecars);
    let _ = fs::remove_dir_all(root);
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
    session.configure_persistence_backends(PersistenceBackends {
        slot: Box::new(FailingSlotBackend),
        autosave: Box::new(FailingSlotBackend),
        mapper: Box::new(FailingSlotBackend),
    });
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
    // The core error type behind the `StateExporter` port still
    // classifies through `CorePersistenceError::slot_failure`.
    assert_eq!(
        CorePersistenceError::WorkerUnavailable.slot_failure(),
        SlotOpFailure::Unavailable
    );
    assert_eq!(
        CorePersistenceError::NoReply.slot_failure(),
        SlotOpFailure::Unavailable
    );
    assert_eq!(
        CorePersistenceError::Core(nerust_core_traits::CoreError::Persistence {
            kind: nerust_core_traits::CoreFault::Incompatible,
            source: "ROM identity mismatch".into(),
        })
        .slot_failure(),
        SlotOpFailure::Incompatible
    );
    assert_eq!(
        CorePersistenceError::Core(nerust_core_traits::CoreError::InvalidCoreOptions)
            .slot_failure(),
        SlotOpFailure::Incompatible
    );
    assert_eq!(
        CorePersistenceError::Core(nerust_core_traits::CoreError::Core(
            "decode error: truncated".into()
        ))
        .slot_failure(),
        SlotOpFailure::Corrupt
    );
    assert_eq!(
        CorePersistenceError::Core(nerust_core_traits::CoreError::Core(
            "system: bus clock ahead of system tick".into()
        ))
        .slot_failure(),
        SlotOpFailure::Corrupt
    );
}

/// Export failures must propagate instead of silently dropping the
/// in-core battery save (registry #1.1b).
struct FailingExportCore;

impl StateExporter for FailingExportCore {
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
        Err(SlotOpFailure::Unavailable)
    }
    fn import_state(&self, _data: Vec<u8>) -> Result<(), SlotOpFailure> {
        Err(SlotOpFailure::Unavailable)
    }
    fn export_preview(&self) -> Option<crate::emu_core::PreviewRgba> {
        None
    }
    fn export_identity(&self) -> Option<nerust_core_traits::identity::SystemIdentity> {
        None
    }
    fn canonical_media_identity(&self) -> Option<nerust_core_traits::identity::SystemIdentity> {
        None
    }
    fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
        Err(SlotOpFailure::Unavailable)
    }
    fn import_mapper(&self, _bytes: Vec<u8>) -> Result<(), SlotOpFailure> {
        Err(SlotOpFailure::Unavailable)
    }
}

#[test]
fn flush_mapper_save_propagates_export_failure() {
    use crate::session::persistence::PersistenceManager;

    let temp_dir = unique_temp_dir("flush-export-failure");
    let mut manager = PersistenceManager::with_all_backends(
        Box::new(FailingSlotBackend),
        Box::new(FailingSlotBackend),
        Box::new(FailingSlotBackend),
    );
    manager.configure(temp_dir.clone().into(), temp_dir.join("test.sav").into());
    assert!(matches!(
        manager.flush_mapper_save(&FailingExportCore),
        Err(crate::session::persistence::MapperFlushError {
            fault: SlotOpFailure::Unavailable
        })
    ));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn setup_failure_pushes_persistence_unavailable() {
    use crate::session::notification::Message;

    // Core active but no ROM loaded: no media identity, so setup fails.
    let mut session = test_session();
    assert_eq!(
        session.setup_persistence(None, MapperRestore::Skip),
        PersistenceSetup::Unavailable
    );
    session.note_persistence_setup(PersistenceSetup::Unavailable);
    let notes = session.drain_notifications();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].message, Message::persistence_unavailable());
}

#[test]
fn pathless_replacement_cannot_write_previous_rom_sidecars() {
    let root = unique_temp_dir("pathless-replacement");
    let mut session = test_session();
    load_test_rom(&mut session, root.join("first.nes"));
    assert!(session.sidecars().is_some());
    let factory = session.active_factory_cloned().unwrap();
    let resolved = factory
        .resolve_load_request(&test_view(&session), factory.default_load_options())
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom_with_mapper4()), resolved)
        .unwrap();
    assert!(session.sidecars().is_none());
    assert!(session.slots().is_empty());
    assert!(!session.save_hidden_lifecycle_state());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execution_hold_survives_load_and_settings_rebuild() {
    use crate::test_helpers::ObservedCoreOptions;
    use nerust_system_traits::factory::load::ResolvedLoadRequest;
    use std::sync::{Arc, Mutex};
    let mut session = test_session();
    let loads = Arc::new(Mutex::new(Vec::new()));
    session.set_execution_held(true);
    session
        .load_resolved(
            MediaObject::new(None, test_rom()),
            ResolvedLoadRequest {
                options: Box::new(ObservedCoreOptions {
                    marker: 73,
                    loads: loads.clone(),
                }),
            },
        )
        .unwrap();
    assert!(session.tick_metrics().paused);
    assert!(
        !session
            .run_command(SessionCommand::Resume)
            .unwrap()
            .executed
    );
    assert!(matches!(
        session.debug_resume(),
        nerust_gui_runtime::debugger::StatusMessage::Paused
    ));
    let output = session.debug_drain(
        &[crate::debug::DebugAction::Resume],
        &crate::debug::DebugDrainInput {
            space: None,
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            reg_target: None,
            freeze: None,
            prev_dump_rows: &[],
            need_images: false,
        },
    );
    assert!(
        output.display.paused,
        "debugger resume must honor the same execution hold"
    );
    let epoch = session.debug_epoch();
    let mut next = session.settings_snapshot().clone();
    next.shared.systems.insert(
        Box::new(DummySystemId),
        Box::new(nerust_nes_settings::NesSettings::default()),
    );
    session.apply_settings(next).unwrap();
    assert!(!epoch.load(std::sync::atomic::Ordering::Acquire));
    assert!(session.tick_metrics().paused);
    assert_eq!(*loads.lock().unwrap(), vec![73, 73]);
    session.set_execution_held(false);
    assert!(
        session
            .run_command(SessionCommand::Resume)
            .unwrap()
            .executed
    );
    assert!(!session.tick_metrics().paused);
}

#[test]
fn every_wiring_funnels_through_configure_backends() {
    use crate::session::fs::{FsAutosave, FsMapper, FsSlots};
    use crate::session::persistence::PersistenceManager;
    let mut manager = PersistenceManager::new();
    manager.with_autosave_backend(Box::new(FsAutosave));
    manager.with_mapper_backend(Box::new(FsMapper));
    manager.configure_backends(PersistenceBackends {
        slot: Box::new(FsSlots),
        autosave: Box::new(FsAutosave),
        mapper: Box::new(FsMapper),
    });
    let bundled = PersistenceManager::with_all_backends(
        Box::new(FsSlots),
        Box::new(FsAutosave),
        Box::new(FsMapper),
    );
    assert!(bundled.slots().is_empty());
    assert!(manager.slots().is_empty());
}

#[test]
fn delete_slot_clears_active_and_selects_neighbours() {
    use crate::session::commands::{SessionCommand, SessionCommandOutcome};
    use crate::session::persistence::SlotDirection;
    let temp_dir = unique_temp_dir("delete-select");
    let mut session = test_session();
    load_test_rom(&mut session, temp_dir.join("test.nes"));
    let saved = session
        .run_command(SessionCommand::SaveActiveSlotOrNew)
        .unwrap();
    assert!(saved.executed);
    let active = session.active_slot_id().expect("save selects a slot");
    let deleted: SessionCommandOutcome = session
        .run_command(SessionCommand::DeleteSlot(active))
        .unwrap();
    assert!(deleted.executed);
    assert_eq!(session.active_slot_id(), None);
    assert_eq!(
        session
            .media
            .persistence
            .select_adjacent_slot(SlotDirection::Next),
        None
    );
    session.media.persistence.select_active_slot(active);
    assert_eq!(session.active_slot_id(), Some(active));
    let _ = fs::remove_dir_all(temp_dir);
}
