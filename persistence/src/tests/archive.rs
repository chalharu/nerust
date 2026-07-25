use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::{prepare_test_dir, test_identity, test_metadata, test_nes_identity};
use crate::{
    archive::build_state_archive,
    metadata::{METADATA_ENTRY, STATE_ARCHIVE_SCHEMA_VERSION, STATE_ENTRY, THUMBNAIL_ENTRY},
    slots::{
        load_state_slot, load_state_slot_for_identity, scan_state_slots, state_slot_path,
        write_state_slot,
    },
    thumbnail::ThumbnailSource,
};

#[derive(serde::Serialize)]
struct LegacyMetadataV2 {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    system_id: String,
    #[serde(with = "serde_bytes")]
    identity_bytes: Vec<u8>,
    #[serde(with = "serde_bytes")]
    options_bytes: Vec<u8>,
    emulator_version: String,
}

#[derive(serde::Serialize)]
struct LegacyMetadataV1 {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    system_id: String,
    mapper_type: u32,
    sub_mapper_type: u32,
    prg_rom_crc64: u64,
    chr_rom_crc64: u64,
    trainer_crc64: u64,
    emulator_version: String,
    rom_format: u32,
    mirror_mode_kind: u32,
    #[serde(with = "serde_bytes")]
    mirror_mode_custom_lut: Vec<u8>,
    has_battery: bool,
    trainer_len: u64,
    prg_rom_len: u64,
    chr_rom_len: u64,
    prg_ram_len: u64,
    save_prg_ram_len: u64,
    chr_ram_len: u64,
    save_chr_ram_len: u64,
}

#[derive(serde::Serialize)]
struct MistaggedSystemIdV3<'a> {
    sid: &'a str,
}

#[derive(serde::Serialize)]
struct MistaggedMetadataV3<'a> {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    system_id: MistaggedSystemIdV3<'a>,
    #[serde(with = "serde_bytes")]
    identity_bytes: Vec<u8>,
    #[serde(with = "serde_bytes")]
    options_bytes: Vec<u8>,
    emulator_version: String,
}

fn write_legacy_archive(path: &std::path::Path, metadata: &impl serde::Serialize) {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(METADATA_ENTRY, options).unwrap();
    writer
        .write_all(&rmp_serde::to_vec_named(metadata).unwrap())
        .unwrap();
    writer.start_file(STATE_ENTRY, options).unwrap();
    writer.write_all(b"legacy-state").unwrap();
    writer.finish().unwrap();
}

#[test]
fn metadata_only_archive_is_not_listed_as_state_slot() {
    let dir = prepare_test_dir("metadata-only-slot");
    let path = state_slot_path(&dir, 3);
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let metadata = test_metadata(3, false);
    writer.start_file(METADATA_ENTRY, options).unwrap();
    writer
        .write_all(&rmp_serde::to_vec_named(&metadata).unwrap())
        .unwrap();
    writer.finish().unwrap();

    assert!(scan_state_slots(&dir).unwrap().is_empty());
    assert!(load_state_slot(&path).is_err());
}

#[test]
fn state_archive_round_trip_preserves_metadata_and_thumbnail() {
    let dir = prepare_test_dir("state-archive-round-trip");

    let summary = write_state_slot(
        &dir,
        7,
        b"machine-state",
        &test_identity(),
        Some(&ThumbnailSource {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        }),
    )
    .unwrap();
    let loaded = load_state_slot(&summary.path).unwrap();

    assert_eq!(loaded.summary.slot_id, 7);
    assert_eq!(loaded.machine_state, b"machine-state");
    assert!(loaded.thumbnail_png.is_some());
    assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
}

#[test]
fn state_archive_reads_known_mistagged_v3_nes_metadata() {
    for (slot_id, tag) in [
        (21, "NesSystemId"),
        (22, "nerust_nes_core::rom_identity::NesSystemId"),
        (23, "nerust_nes_core::nes"),
    ] {
        let dir = prepare_test_dir(&format!("state-archive-v3-mistagged-{slot_id}"));
        let path = state_slot_path(&dir, slot_id);
        write_legacy_archive(
            &path,
            &MistaggedMetadataV3 {
                schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
                slot_id,
                saved_at_unix_ms: 1234,
                has_thumbnail: false,
                system_id: MistaggedSystemIdV3 { sid: tag },
                identity_bytes: vec![1, 2, 3, 4],
                options_bytes: vec![5, 6],
                emulator_version: "mistagged-v3".into(),
            },
        );

        let loaded = load_state_slot_for_identity(&path, &test_nes_identity())
            .unwrap()
            .expect("known mistagged NES metadata should match the current NES identity");
        assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
        assert_eq!(loaded.summary.slot_id, slot_id);
        assert_eq!(loaded.summary.emulator_version, "mistagged-v3");
        assert_eq!(loaded.machine_state, b"legacy-state");
    }
}

#[test]
fn state_archive_rejects_unknown_mistagged_v3_metadata() {
    let dir = prepare_test_dir("state-archive-v3-unknown-tag");
    let path = state_slot_path(&dir, 24);
    write_legacy_archive(
        &path,
        &MistaggedMetadataV3 {
            schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
            slot_id: 24,
            saved_at_unix_ms: 1234,
            has_thumbnail: false,
            system_id: MistaggedSystemIdV3 {
                sid: "future_system::SystemId",
            },
            identity_bytes: vec![1, 2, 3, 4],
            options_bytes: Vec::new(),
            emulator_version: "unknown-v3".into(),
        },
    );

    assert!(load_state_slot(&path).is_err());
}

#[test]
fn state_archive_reads_v2_string_system_id_metadata() {
    let dir = prepare_test_dir("state-archive-v2");
    let path = state_slot_path(&dir, 12);
    write_legacy_archive(
        &path,
        &LegacyMetadataV2 {
            schema_version: 2,
            slot_id: 12,
            saved_at_unix_ms: 1234,
            has_thumbnail: false,
            system_id: "nes".into(),
            identity_bytes: vec![1, 2, 3, 4],
            options_bytes: vec![5, 6],
            emulator_version: "v2".into(),
        },
    );

    let loaded = load_state_slot(&path).unwrap();
    assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
    assert_eq!(loaded.summary.slot_id, 12);
    assert_eq!(loaded.summary.emulator_version, "v2");
    assert_eq!(loaded.machine_state, b"legacy-state");
    assert!(
        load_state_slot_for_identity(&path, &test_nes_identity())
            .unwrap()
            .is_some()
    );
}

#[test]
fn state_archive_rejects_non_nes_legacy_metadata() {
    let dir = prepare_test_dir("state-archive-v2-non-nes");
    let path = state_slot_path(&dir, 13);
    write_legacy_archive(
        &path,
        &LegacyMetadataV2 {
            schema_version: 2,
            slot_id: 13,
            saved_at_unix_ms: 1234,
            has_thumbnail: false,
            system_id: "snes".into(),
            identity_bytes: vec![1, 2, 3, 4],
            options_bytes: Vec::new(),
            emulator_version: "v2".into(),
        },
    );

    let error = load_state_slot(&path).expect_err("non-NES legacy metadata should reject");
    assert!(
        error
            .to_string()
            .contains("unsupported legacy state archive system: snes")
    );
}

#[test]
fn state_archive_reads_v1_string_system_id_metadata() {
    let dir = prepare_test_dir("state-archive-v1");
    let path = state_slot_path(&dir, 11);
    write_legacy_archive(
        &path,
        &LegacyMetadataV1 {
            schema_version: 1,
            slot_id: 11,
            saved_at_unix_ms: 1234,
            has_thumbnail: false,
            system_id: "Nes".into(),
            mapper_type: 4,
            sub_mapper_type: 1,
            prg_rom_crc64: 10,
            chr_rom_crc64: 20,
            trainer_crc64: 30,
            emulator_version: "v1".into(),
            rom_format: 1,
            mirror_mode_kind: 5,
            mirror_mode_custom_lut: vec![0, 1, 1, 0],
            has_battery: true,
            trainer_len: 512,
            prg_rom_len: 32768,
            chr_rom_len: 8192,
            prg_ram_len: 8192,
            save_prg_ram_len: 8192,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
        },
    );

    let loaded = load_state_slot(&path).unwrap();
    assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
    assert_eq!(loaded.summary.slot_id, 11);
    assert_eq!(loaded.summary.emulator_version, "v1");
    assert_eq!(loaded.machine_state, b"legacy-state");
}

#[test]
fn state_archive_rejects_schema_mismatch() {
    let dir = prepare_test_dir("schema-mismatch");
    let path = state_slot_path(&dir, 1);
    let mut metadata = test_metadata(1, false);
    metadata.schema_version = STATE_ARCHIVE_SCHEMA_VERSION + 1;
    let archive = build_state_archive(&metadata, b"state", None).unwrap();
    fs::write(&path, archive).unwrap();

    let error = load_state_slot(&path).expect_err("schema mismatch should reject");
    assert!(
        error
            .to_string()
            .contains("unsupported state archive schema version")
    );
}

#[test]
fn missing_thumbnail_is_reported_consistently_even_if_metadata_claims_presence() {
    let dir = prepare_test_dir("missing-thumbnail");
    let path = state_slot_path(&dir, 4);
    let metadata = test_metadata(4, true);
    fs::write(
        &path,
        build_state_archive(&metadata, b"state", None).unwrap(),
    )
    .unwrap();

    let summary = scan_state_slots(&dir).unwrap().pop().unwrap();
    let loaded = load_state_slot(&path).unwrap();
    assert!(!summary.has_thumbnail);
    assert!(!loaded.summary.has_thumbnail);
    assert_eq!(loaded.thumbnail_png, None);
}

#[test]
fn invalid_thumbnail_bytes_are_preserved_as_opaque_blob() {
    let dir = prepare_test_dir("invalid-thumbnail");
    let path = state_slot_path(&dir, 8);
    let metadata = test_metadata(8, true);
    let cursor = Cursor::new(Vec::<u8>::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(METADATA_ENTRY, options).unwrap();
    writer
        .write_all(&rmp_serde::to_vec_named(&metadata).unwrap())
        .unwrap();
    writer.start_file(STATE_ENTRY, options).unwrap();
    writer.write_all(b"state").unwrap();
    writer.start_file(THUMBNAIL_ENTRY, options).unwrap();
    writer
        .write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF])
        .unwrap();
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();

    let summary = scan_state_slots(&dir).unwrap().pop().unwrap();
    let loaded = load_state_slot(&path).unwrap();
    assert!(summary.has_thumbnail);
    assert!(loaded.summary.has_thumbnail);
    assert_eq!(
        loaded.thumbnail_png,
        Some(vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF])
    );
}
