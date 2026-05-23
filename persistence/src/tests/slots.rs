use super::support::{prepare_test_dir, test_rom_identity, test_target};
use crate::{
    ThumbnailSource, allocate_next_slot_id, delete_state_slot, load_state_slot, scan_state_slots,
    scan_state_slots_for_target, state_slot_path, write_state_slot,
};
use nerust_contract::{CoreOptions, Mmc3IrqVariant, PersistenceTarget, RomFormat, RomIdentity};
use std::fs;

#[test]
fn slot_id_allocation_is_monotonic_across_deletions() {
    let dir = prepare_test_dir("slot-id-allocation");

    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
    write_state_slot(&dir, 1, b"a", test_target(), None).unwrap();
    write_state_slot(&dir, 2, b"b", test_target(), None).unwrap();
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
fn corrupt_slot_does_not_hide_valid_slots_or_block_allocation() {
    let dir = prepare_test_dir("corrupt-slot-scan");

    write_state_slot(&dir, 1, b"ok", test_target(), None).unwrap();
    fs::write(state_slot_path(&dir, 2), b"not-a-zip-archive").unwrap();

    let slots = scan_state_slots(&dir).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot_id, 1);
    assert_eq!(allocate_next_slot_id(&dir).unwrap(), 3);
}

#[test]
fn scan_state_slots_for_target_filters_incompatible_slots() {
    let dir = prepare_test_dir("target-filtered-slots");
    let matching_identity = test_rom_identity();
    let mismatched_identity = RomIdentity {
        prg_rom_crc64: 100,
        ..matching_identity
    };
    let header_corrected_identity = RomIdentity {
        format: RomFormat::Nes20,
        ..matching_identity
    };

    write_state_slot(
        &dir,
        1,
        b"matching",
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        2,
        b"mismatched-rom",
        PersistenceTarget {
            rom_identity: mismatched_identity,
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        3,
        b"mismatched-options",
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        4,
        b"header-corrected",
        PersistenceTarget {
            rom_identity: header_corrected_identity,
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();

    let slots = scan_state_slots_for_target(
        &dir,
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions::default(),
        },
    )
    .unwrap();
    let slot_ids = slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>();

    assert_eq!(slot_ids, vec![1]);
}

#[test]
fn strict_target_filtering_requires_all_identity_and_option_fields() {
    let dir = prepare_test_dir("strict-target-filtering");
    let matching_identity = test_rom_identity();
    write_state_slot(
        &dir,
        1,
        b"matching",
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        2,
        b"battery-mismatch",
        PersistenceTarget {
            rom_identity: RomIdentity {
                has_battery: false,
                ..matching_identity
            },
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        3,
        b"save-ram-mismatch",
        PersistenceTarget {
            rom_identity: RomIdentity {
                save_prg_ram_len: matching_identity.save_prg_ram_len + 1,
                ..matching_identity
            },
            options: CoreOptions::default(),
        },
        None,
    )
    .unwrap();
    write_state_slot(
        &dir,
        4,
        b"option-mismatch",
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            },
        },
        None,
    )
    .unwrap();

    let slots = scan_state_slots_for_target(
        &dir,
        PersistenceTarget {
            rom_identity: matching_identity,
            options: CoreOptions::default(),
        },
    )
    .unwrap();
    assert_eq!(
        slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn save_slot_summary_matches_loaded_summary() {
    let dir = prepare_test_dir("summary-consistency");
    let written = write_state_slot(
        &dir,
        11,
        b"state",
        test_target(),
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
