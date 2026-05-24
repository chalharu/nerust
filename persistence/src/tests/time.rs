use crate::metadata::STATE_ARCHIVE_SCHEMA_VERSION;
use crate::model::StateSlotSummary;
use crate::time::{format_slot_saved_at, latest_saved_slot_id};
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn slot_timestamp_format_is_human_readable() {
    let formatted = format_slot_saved_at(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    assert_eq!(formatted.len(), 19);
    assert_eq!(formatted.chars().nth(4), Some('-'));
    assert_eq!(formatted.chars().nth(7), Some('-'));
    assert_eq!(formatted.chars().nth(10), Some(' '));
    assert_eq!(formatted.chars().nth(13), Some(':'));
    assert_eq!(formatted.chars().nth(16), Some(':'));
}

#[test]
fn latest_saved_slot_id_prefers_newest_timestamp_then_slot_id() {
    let summary = |slot_id, saved_at_secs| StateSlotSummary {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        path: PathBuf::from(format!("/tmp/{slot_id}.state")),
        saved_at: UNIX_EPOCH + Duration::from_secs(saved_at_secs),
        has_thumbnail: false,
        emulator_version: "test".into(),
    };

    let slots = vec![
        summary(2, 10),
        summary(7, 20),
        summary(9, 20),
        summary(4, 15),
    ];

    assert_eq!(latest_saved_slot_id(&slots), Some(9));
    assert_eq!(latest_saved_slot_id(&[]), None);
}
