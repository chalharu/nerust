//! OSD history formatting: level text plus render-ready rows.
//! Pure mapping from notifications to grid lines; no queue state.

use nerust_settings_core::model::language::AppLanguage;

use crate::session::notification::{NotificationLevel, NotificationRenderer, UiNotification};

/// Level tag prefixing OSD labels: text carries severity; grid names come from [`level_name`].
/// Single source for level text so a new level cannot update one side and forget the other.
struct LevelText {
    tag: &'static str,
    name: &'static str,
}

fn level_text(level: NotificationLevel) -> LevelText {
    match level {
        NotificationLevel::Info => LevelText {
            tag: "[INFO]",
            name: "Info",
        },
        NotificationLevel::Warning => LevelText {
            tag: "[WARN]",
            name: "Warning",
        },
        NotificationLevel::Error => LevelText {
            tag: "[ERROR]",
            name: "Error",
        },
    }
}

pub fn level_tag(level: NotificationLevel) -> &'static str {
    level_text(level).tag
}

/// Full level name for the history grid Level column.
pub fn level_name(level: NotificationLevel) -> &'static str {
    level_text(level).name
}

/// Localized level name for history grid and filter options.
/// `Error` maps to `UiText::Error` for column width.
pub fn level_label(
    level: NotificationLevel,
    language: nerust_settings_core::model::language::AppLanguage,
) -> &'static str {
    NotificationRenderer { language }.level_name(level)
}

/// One rendered history line: datetime plus grid columns.
/// `module` is the log module so rows grep the `notification:` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLine {
    /// YYYY-MM-DD HH:MM:SS (local wall-clock) of the push.
    pub datetime: String,
    pub level: NotificationLevel,
    pub module: &'static str,
    pub text: String,
    pub count: u32,
}

/// Local wall-clock datetime from a `SystemTime`; falls back to UTC when the zone is indeterminate.
pub fn format_datetime(time: std::time::SystemTime) -> String {
    format_datetime_in(time, local_utc_offset_secs())
}

/// `SystemTime` as local epoch seconds for [`format_datetime`]: UTC instant plus platform offset.
fn local_epoch_secs(time: std::time::SystemTime) -> i64 {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (utc as i64).saturating_add(local_utc_offset_secs())
}

/// Platform zone offset in seconds, resolved once per sync — not per
/// row. Falls back to UTC when the zone is indeterminate.
fn local_utc_offset_secs() -> i64 {
    time::UtcOffset::current_local_offset()
        .map(|o| i64::from(o.whole_seconds()))
        .unwrap_or(0)
}

/// Zero UTC offset for civil-math normalization; the conversion is infallible by construction.
const ZERO_OFFSET_SECS: i32 = 0;

/// Datetime with a pre-resolved offset; out-of-range instants fall back to the epoch rendering.
fn format_datetime_in(time: std::time::SystemTime, offset_secs: i64) -> String {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = utc.saturating_add(offset_secs);
    debug_assert!(time::UtcOffset::from_whole_seconds(ZERO_OFFSET_SECS).is_ok());
    let offset =
        time::UtcOffset::from_whole_seconds(ZERO_OFFSET_SECS).unwrap_or(time::UtcOffset::UTC);
    if let Ok(dt) = time::OffsetDateTime::from_unix_timestamp(secs) {
        let dt = dt.to_offset(offset);
        let (year, month, day) = dt.to_calendar_date();
        let (h, m, s) = dt.to_hms();
        format!(
            "{year:04}-{:02}-{:02} {h:02}:{m:02}:{s:02}",
            u8::from(month),
            day,
        )
    } else {
        "1970-01-01 00:00:00".to_string()
    }
}

/// Build render-ready history lines in one pass, resolving the zone offset once for all rows.
pub fn history_lines<'a>(
    history: impl Iterator<Item = &'a UiNotification>,
    language: AppLanguage,
) -> Vec<HistoryLine> {
    let offset_secs = local_utc_offset_secs();
    history
        .map(|n| {
            let text = super::notification::render_text(&n.kind, n.count, language);
            HistoryLine {
                datetime: format_datetime_in(n.at, offset_secs),
                level: n.level,
                module: super::notification::LOG_MODULE,
                text,
                count: n.count,
            }
        })
        .collect()
}

pub fn format_time(time: std::time::SystemTime) -> String {
    let clock = local_epoch_secs(time).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::notification::{
        FailureOrigin, NotificationKind, NotificationLevel, UiNotification,
    };
    use nerust_settings_core::model::language::AppLanguage;

    #[test]
    fn level_tag_and_name_stay_distinct() {
        assert_eq!(level_tag(NotificationLevel::Warning), "[WARN]");
        assert_eq!(level_name(NotificationLevel::Warning), "Warning");
        assert_eq!(level_name(NotificationLevel::Error), "Error");
    }

    #[test]
    fn level_names_stay_distinct_per_surface() {
        assert_eq!(level_tag(NotificationLevel::Info), "[INFO]");
        assert_eq!(level_name(NotificationLevel::Info), "Info");
        assert_eq!(
            level_label(NotificationLevel::Info, AppLanguage::English),
            "Info"
        );
        assert_eq!(
            level_label(NotificationLevel::Warning, AppLanguage::Japanese),
            "警告"
        );
        assert_eq!(
            level_label(NotificationLevel::Error, AppLanguage::English),
            "Error"
        );
        assert_eq!(
            level_label(NotificationLevel::Error, AppLanguage::Japanese),
            "エラー"
        );
    }

    #[test]
    fn history_datetime_is_local_wall_clock() {
        // Only this test writes TZ; the rest assert TZ-free structure for parallel soundness.
        // SAFETY: single-process test env, no other TZ writer.
        unsafe { std::env::set_var("TZ", "UTC0") };
        let epoch = std::time::UNIX_EPOCH;
        assert_eq!(format_datetime(epoch), "1970-01-01 00:00:00");
        assert_eq!(format_time(epoch), "00:00:00");
        let known = epoch + std::time::Duration::from_secs(1_769_990_400);
        assert_eq!(format_datetime(known), "2026-02-02 00:00:00");
    }

    #[test]
    fn history_datetime_keeps_fixed_shape_and_order() {
        let epoch = std::time::UNIX_EPOCH;
        let rendered = format_datetime(epoch);
        assert_eq!(rendered.len(), 19);
        assert_eq!(&rendered[4..5], "-");
        assert_eq!(&rendered[10..11], " ");
        assert_eq!(&rendered[13..14], ":");
        // A full day always advances wall-clock, in any zone.
        let next = format_datetime(epoch + std::time::Duration::from_secs(86_400));
        assert!(rendered < next);
    }

    #[test]
    fn history_lines_batch_matches_single_formatting() {
        // Checked structurally: one row per event, ×N suffix preserved, level carried through.
        let events = [
            UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            },
            UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::CommandFailure {
                    origin: FailureOrigin::Command,
                    id: "Save",
                    detail: String::new(),
                },
                count: 3,
                seq: 1,
                at: std::time::UNIX_EPOCH,
            },
        ];
        let rows = history_lines(events.iter(), AppLanguage::English);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].datetime, format_datetime(std::time::UNIX_EPOCH));
        assert_eq!(rows[0].text, "Paused");
        assert!(rows[1].text.ends_with(" ×3"));
        assert_eq!(rows[1].count, 3);
    }
}
