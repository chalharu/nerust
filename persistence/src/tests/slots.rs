use std::fs;

use super::{prepare_test_dir, test_identity, test_identity_with_bytes};
use crate::{
    slots::{
        allocate_next_slot_id, autosave_state_slot_path, delete_state_slot, load_state_slot,
        scan_state_slots, scan_state_slots_for_identity, state_slot_path,
        write_autosave_state_slot, write_state_slot,
    },
    thumbnail::ThumbnailSource,
};

#[test]
fn slot_id_allocation_is_monotonic_across_deletions() {
    let dir = prepare_test_dir("slot-id-allocation");

    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
    write_state_slot(&dir, 1, b"a", &test_identity(), None).unwrap();
    write_state_slot(&dir, 2, b"b", &test_identity(), None).unwrap();
    delete_state_slot(&state_slot_path(&dir, 1)).unwrap();

    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 3);
}

#[test]
fn slot_id_allocation_persists_without_writing_slot_files() {
    let dir = prepare_test_dir("slot-id-counter");

    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 2);
}

#[test]
fn autosave_slot_round_trips_without_listing_or_allocation() {
    let dir = prepare_test_dir("autosave-slot");

    let written = write_autosave_state_slot(&dir, b"autosave", &test_identity(), None).unwrap();
    let loaded = load_state_slot(&autosave_state_slot_path(&dir)).unwrap();

    assert_eq!(written.slot_id, 0);
    assert_eq!(loaded.summary, written);
    assert_eq!(loaded.machine_state, b"autosave");
    assert!(scan_state_slots(&dir).unwrap().is_empty());
    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
}

#[test]
fn corrupt_slot_does_not_hide_valid_slots_or_block_allocation() {
    let dir = prepare_test_dir("corrupt-slot-scan");

    write_state_slot(&dir, 1, b"ok", &test_identity(), None).unwrap();
    fs::write(state_slot_path(&dir, 2), b"not-a-zip-archive").unwrap();

    let slots = scan_state_slots(&dir).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot_id, 1);
    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 3);
}

#[test]
fn scan_state_slots_for_identity_filters_different_roms() {
    let dir = prepare_test_dir("identity-filtered-slots");
    let matching = test_identity();
    let mismatched = test_identity_with_bytes(vec![5, 6, 7, 8]);

    write_state_slot(
        &dir,
        1,
        b"matching",
        &test_identity_with_bytes(matching.identity_bytes.clone()),
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        2,
        b"mismatched-rom",
        &test_identity_with_bytes(mismatched.identity_bytes.clone()),
        None,
    )
    .unwrap();

    let slots = scan_state_slots_for_identity(
        &dir,
        &test_identity_with_bytes(matching.identity_bytes.clone()),
    )
    .unwrap();
    let slot_ids = slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>();

    assert_eq!(slot_ids, vec![1]);
}

#[test]
fn identity_filtering_keeps_canonical_matches_across_runtime_options() {
    let dir = prepare_test_dir("canonical-identity-filtering");
    let matching = test_identity();

    write_state_slot(
        &dir,
        1,
        b"matching",
        &test_identity_with_bytes(matching.identity_bytes.clone()),
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        2,
        b"mismatched",
        &test_identity_with_bytes(vec![9, 10, 11, 12]),
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        3,
        b"same-identity-again",
        &test_identity_with_bytes(matching.identity_bytes.clone()),
        None,
    )
    .unwrap();

    let slots = scan_state_slots_for_identity(
        &dir,
        &test_identity_with_bytes(matching.identity_bytes.clone()),
    )
    .unwrap();
    assert_eq!(
        slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>(),
        vec![1, 3]
    );
}

#[test]
fn save_slot_summary_matches_loaded_summary() {
    let dir = prepare_test_dir("summary-consistency");
    let written = write_state_slot(
        &dir,
        11,
        b"state",
        &test_identity(),
        Some(&ThumbnailSource {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        }),
    )
    .unwrap();

    let mut scanned_slots = scan_state_slots(&dir).unwrap();
    let scanned = scanned_slots.pop().unwrap();
    let loaded = load_state_slot(&written.path).unwrap();
    assert_eq!(written.saved_at, scanned.saved_at);
    assert_eq!(scanned.schema_version, written.schema_version);
    assert_eq!(scanned.slot_id, written.slot_id);
    assert_eq!(scanned.path, written.path);
    assert_eq!(scanned.has_thumbnail, written.has_thumbnail);
    assert_eq!(scanned.emulator_version, written.emulator_version);
    assert_eq!(loaded.summary, scanned);
}
