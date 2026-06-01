use crate::error::PersistenceError;
use crate::model::StateSlotSummary;
use libc::tm;
use std::mem::MaybeUninit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn format_slot_saved_at(saved_at: SystemTime) -> String {
    let Ok(duration) = saved_at.duration_since(UNIX_EPOCH) else {
        return "unknown".into();
    };

    format_local_timestamp(duration.as_secs() as i64)
        .unwrap_or_else(|| duration.as_secs().to_string())
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

fn format_local_timestamp(epoch_seconds: i64) -> Option<String> {
    let tm = localtime(epoch_seconds)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    ))
}

#[cfg(unix)]
fn localtime(epoch_seconds: i64) -> Option<tm> {
    let time = epoch_seconds;
    let mut result = MaybeUninit::<tm>::uninit();
    unsafe {
        if libc::localtime_r(&time, result.as_mut_ptr()).is_null() {
            None
        } else {
            Some(result.assume_init())
        }
    }
}

#[cfg(windows)]
fn localtime(epoch_seconds: i64) -> Option<tm> {
    let time = epoch_seconds;
    let mut result = MaybeUninit::<tm>::uninit();
    unsafe {
        if libc::localtime_s(result.as_mut_ptr(), &time) != 0 {
            None
        } else {
            Some(result.assume_init())
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn localtime(_epoch_seconds: i64) -> Option<tm> {
    None
}
