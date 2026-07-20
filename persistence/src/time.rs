use std::time::{Duration, SystemTime, UNIX_EPOCH};

use time::{OffsetDateTime, macros::format_description};

use crate::{error::PersistenceError, model::StateSlotSummary};

const DATE_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

pub fn format_slot_saved_at(saved_at: SystemTime) -> String {
    let Ok(duration) = saved_at.duration_since(UNIX_EPOCH) else {
        return "unknown".into();
    };

    let epoch_seconds = duration.as_secs() as i64;
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(epoch_seconds) else {
        return epoch_seconds.to_string();
    };
    let Ok(offset) = time::UtcOffset::current_local_offset() else {
        return epoch_seconds.to_string();
    };
    dt.to_offset(offset)
        .format(DATE_TIME_FORMAT)
        .unwrap_or_else(|_| epoch_seconds.to_string())
}

pub fn latest_saved_slot_id(slots: &[StateSlotSummary]) -> Option<u64> {
    slots
        .iter()
        .max_by_key(|slot| {
            (
                slot.saved_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO),
                slot.slot_id,
            )
        })
        .map(|slot| slot.slot_id)
}

pub(crate) fn unix_millis(time: SystemTime) -> Result<u64, PersistenceError> {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PersistenceError::Validation(error.to_string()))?
        .as_millis();
    u64::try_from(millis)
        .map_err(|_| PersistenceError::Validation("timestamp overflows u64 milliseconds".into()))
}

pub(crate) fn system_time_from_millis(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}
