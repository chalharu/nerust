use super::{prepare_test_dir, test_identity, test_metadata};
use crate::archive::build_state_archive;
use crate::metadata::{METADATA_ENTRY, STATE_ARCHIVE_SCHEMA_VERSION, STATE_ENTRY, THUMBNAIL_ENTRY};
use crate::slots::{load_state_slot, scan_state_slots, state_slot_path, write_state_slot};
use crate::thumbnail::ThumbnailSource;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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
