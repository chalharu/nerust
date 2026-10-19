//! OSD history formatting: level text plus render-ready rows.
//! Pure mapping from notifications to grid lines; no queue state.

use nerust_settings_core::model::language::AppLanguage;

use crate::session::notification::{NotificationLevel, UiNotification};

/// Level tag prefixing OSD labels: text (not color or shape
/// alone) carries severity. History uses [`level_name`]
/// instead: the grid Level column and the OSD tag are managed
/// separately by design (review feedback).
/// Single source for level text: tag and grid name side by
/// side so a new level cannot update one and forget the other.
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
    use nerust_settings_core::i18n::{UiText, text as ui_text};
    match level {
        NotificationLevel::Info => ui_text(language, UiText::NotificationInfo),
        NotificationLevel::Warning => ui_text(language, UiText::NotificationWarning),
        NotificationLevel::Error => ui_text(language, UiText::Error),
    }
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

/// Local wall-clock datetime from a `SystemTime`.
/// Falls back to UTC when the zone is indeterminate.
pub fn format_datetime(time: std::time::SystemTime) -> String {
    format_datetime_in(time, local_utc_offset_secs())
}

/// `SystemTime` as local epoch seconds for [`format_datetime`]:
/// the UTC instant plus the platform zone offset. Mirrors the
/// persistence slot-time precedent (`persistence::time`).
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

/// Datetime with a pre-resolved offset (see
/// [`local_utc_offset_secs`]). Civil math delegates to `time`;
/// out-of-range instants fall back to the epoch rendering.
fn format_datetime_in(time: std::time::SystemTime, offset_secs: i64) -> String {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = utc.saturating_add(offset_secs);
    let offset = time::UtcOffset::from_whole_seconds(0).expect("zero offset valid");
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

/// Build render-ready history lines in one pass, resolving the zone
/// offset once for all rows. Lifecycle delegates here so every
/// frontend shares identical rows without duplicating the mapping.
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
