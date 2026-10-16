//! OSD display state shared by frontends: queue folding, expiry,
//! burst caps, strip math, and modal aggregation. Frontends render
//! `rows()` and honor `next_deadline()`; all policy lives here so
//! tao/gtk/android behave identically.

use std::time::{Duration, Instant};

use nerust_settings_core::model::language::AppLanguage;
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, NotificationSettings,
};

use super::notification::{NotificationLevel, UiNotification};

/// Level tag prefixing OSD labels: text (not color or shape
/// alone) carries severity. History uses [`level_name`]
/// instead: the grid Level column and the OSD tag are managed
/// separately by design (review feedback).
pub fn level_tag(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "[INFO]",
        NotificationLevel::Warning => "[WARN]",
        NotificationLevel::Error => "[ERROR]",
    }
}

/// Full level name for the history grid Level column.
pub fn level_name(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "Info",
        NotificationLevel::Warning => "Warning",
        NotificationLevel::Error => "Error",
    }
}

/// Localized level name for display surfaces (history grid, filter
/// options). The canonical English [`level_name`] stays for logs and
/// tests; UI text goes through the single i18n table. Note `Error`
/// maps to `UiText::Error` ("Error"), not `NotificationError`
/// ("Error notifications"), to match the column width and siblings.
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

/// Render contract handed to frontends: rows plus status placement.
#[derive(Debug, Clone, Default)]
pub struct OsdSnapshot {
    pub rows: Vec<OsdRow>,
    pub status: Option<String>,
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
    /// Logical pixels (viewport-relative sizing applied by frontend).
    pub font_px: f32,
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

/// Local wall-clock datetime from a `SystemTime`: the UTC instant
/// shifted by the platform zone offset, then rendered with the same
/// dependency-free civil math. History shows time where the user
/// sits (like Console/Event Viewer); UTC-only stays in the logs.
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
/// [`local_utc_offset_secs`]).
fn format_datetime_in(time: std::time::SystemTime, offset_secs: i64) -> String {
    let utc = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Floor-divide so pre-1970 instants keep a non-negative clock.
    let secs = (utc as i64).saturating_add(offset_secs);
    let days = secs.div_euclid(86_400);
    let clock = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days; 719468 = days to 1970-01-01.
    let era = (days + 719_468) / 146_097;
    let doe = (days + 719_468) - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let full_year = (year + i64::from(month <= 2)) as u32;
    format!(
        "{full_year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
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
            let mut text = n.kind.text(language);
            if n.count > 1 {
                text = format!("{text} ×{}", n.count);
            }
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

#[derive(Debug, Clone)]
pub struct OsdRow {
    pub text: String,
    pub rgb: (u8, u8, u8),
    pub tag: &'static str,
}

#[derive(Debug)]
struct OsdItem {
    row: OsdRow,
    level: NotificationLevel,
    /// Identity for coalescing (text before the `×N` suffix).
    key: String,
    count: u32,
    deadline: Instant,
}

/// Single sequential display (no stacking): one label at a time,
/// FIFO by push order. `pending` is capped at two so the total
/// (current + pending) never exceeds three.
const MAX_QUEUED: usize = 2;

#[derive(Debug, Default)]
pub struct OsdManager {
    current: Option<OsdItem>,
    pending: std::collections::VecDeque<OsdItem>,
    modal: Vec<String>,
    modal_queue: std::collections::VecDeque<String>,
    modal_open: bool,
    dirty: bool,
}

impl OsdManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(level: NotificationLevel, settings: &NotificationSettings) -> bool {
        match level {
            NotificationLevel::Info => settings.info_enabled,
            NotificationLevel::Warning => settings.warning_enabled,
            NotificationLevel::Error => settings.error_enabled,
        }
    }

    fn duration_ms(level: NotificationLevel, settings: &NotificationSettings) -> u64 {
        let raw = match level {
            NotificationLevel::Info => settings.info_duration_ms,
            NotificationLevel::Warning => settings.warning_duration_ms,
            NotificationLevel::Error => 5_000,
        };
        u64::from(raw.clamp(500, 60_000))
    }

    fn counted_row(base: &str, count: u32, rgb: (u8, u8, u8), tag: &'static str) -> OsdRow {
        let text = if count > 1 {
            format!("{base} ×{count}")
        } else {
            base.to_string()
        };
        OsdRow { text, rgb, tag }
    }

    fn color(level: NotificationLevel, settings: &NotificationSettings) -> (u8, u8, u8) {
        match level {
            NotificationLevel::Info => settings.info_color.rgb(),
            NotificationLevel::Warning => settings.warning_color.rgb(),
            NotificationLevel::Error => settings.error_color.rgb(),
        }
    }

    /// Fold drained shell notifications into display state.
    /// Errors go modal (aggregated) or OSD (minimum 5s) per settings.
    /// New arrivals queue behind the showing label; identical text
    /// merges with the showing label or the queue tail (`×N`, timer reset).
    pub fn push_all(
        &mut self,
        notifications: Vec<UiNotification>,
        settings: &NotificationSettings,
        language: AppLanguage,
        now: Instant,
    ) {
        if !notifications.is_empty() {
            self.dirty = true;
        }
        for n in notifications {
            if !Self::enabled(n.level, settings) {
                continue;
            }
            let base = n.kind.text(language);
            let mut text = base.clone();
            if n.count > 1 {
                text = format!("{text} ×{}", n.count);
            }
            if n.level == NotificationLevel::Error && settings.error_modal {
                self.modal.push(text);
                continue;
            }
            let rgb = Self::color(n.level, settings);
            let tag = level_tag(n.level);
            let deadline = now + Duration::from_millis(Self::duration_ms(n.level, settings));
            // Merge with the showing label when identical.
            if let Some(current) = self.current.as_mut()
                && current.level == n.level
                && current.key == base
            {
                current.count += n.count;
                current.row = Self::counted_row(&base, current.count, rgb, tag);
                current.deadline = deadline;
                continue;
            }
            // Merge with the queue tail when identical.
            if let Some(back) = self.pending.back_mut()
                && back.level == n.level
                && back.key == base
            {
                back.count += n.count;
                back.row = Self::counted_row(&base, back.count, rgb, tag);
                back.deadline = deadline;
                continue;
            }
            let row = Self::counted_row(&base, n.count, rgb, tag);
            self.pending.push_back(OsdItem {
                row,
                level: n.level,
                key: base,
                count: n.count,
                deadline,
            });
            // Never preempt the showing label; drop oldest pending.
            while self.pending.len() > MAX_QUEUED {
                self.pending.pop_front();
            }
        }
    }

    /// Advance the showing label. Promotes pending immediately when
    /// idle; advances on expiry otherwise. Returns true on change.
    pub fn poll(&mut self, now: Instant) -> bool {
        let need = match &self.current {
            None => !self.pending.is_empty(),
            Some(current) => current.deadline <= now,
        };
        if !need {
            return false;
        }
        self.current = self.pending.pop_front();
        self.dirty = true;
        true
    }

    /// True when display state changed since the last sync.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// The single showing row (empty when idle).
    pub fn rows(&self) -> Vec<OsdRow> {
        self.current
            .as_ref()
            .map(|current| vec![current.row.clone()])
            .unwrap_or_default()
    }

    pub fn take_modal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.modal)
    }

    /// Queue one aggregated error batch for sequential display.
    /// Capped: a stuck-open dialog plus a per-tick error flood must
    /// not grow memory without bound. Drops the oldest, like the OSD
    /// pending cap, and logs the loss (history keeps every event).
    pub fn queue_modal(&mut self, detail: String) {
        const MAX_MODAL_QUEUED: usize = 8;
        if self.modal_queue.len() >= MAX_MODAL_QUEUED {
            log::warn!("error modal queue full; dropping oldest batch");
            self.modal_queue.pop_front();
        }
        self.modal_queue.push_back(detail);
    }

    /// Take the next batch only when no dialog is showing: frontends
    /// present one dialog at a time and call [`OsdManager::modal_closed`]
    /// on ack, so errors never stack.
    pub fn take_modal_batch(&mut self) -> Option<String> {
        if self.modal_open {
            return None;
        }
        let next = self.modal_queue.pop_front()?;
        self.modal_open = true;
        Some(next)
    }

    pub fn modal_closed(&mut self) {
        self.modal_open = false;
    }

    pub fn modal_pending(&self) -> usize {
        self.modal_queue.len()
    }

    pub fn has_visible(&self) -> bool {
        self.current.is_some()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match &self.current {
            Some(current) => Some(current.deadline),
            // Pending work with nothing showing: wake immediately.
            None if !self.pending.is_empty() => Some(Instant::now()),
            None => None,
        }
    }

    pub fn status_text(fps: f32, paused: bool, language: AppLanguage) -> String {
        if paused {
            match language {
                AppLanguage::Japanese => format!("一時停止中 {fps:.1} fps"),
                _ => format!("Paused {fps:.1} fps"),
            }
        } else {
            format!("{fps:.1} fps")
        }
    }
}

/// Strip rows per edge from transient + status corners.
/// Same edge, different sides share one strip; same corner stacks two.
pub fn strip_rows(
    area_mode: NotificationAreaMode,
    transient: NotificationPosition,
    status: NotificationPosition,
    status_on: bool,
) -> (u32, u32) {
    if !matches!(area_mode, NotificationAreaMode::Strip) {
        return (0, 0);
    }
    fn edge(position: NotificationPosition) -> bool {
        matches!(
            position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        )
    }
    let mut top = [0u32, 0u32];
    let mut bottom = [0u32, 0u32];
    let sides = [(transient, true), (status, status_on)];
    for (position, on) in sides {
        if !on {
            continue;
        }
        let side = usize::from(matches!(
            position,
            NotificationPosition::TopRight | NotificationPosition::BottomRight
        ));
        if edge(position) {
            top[side] += 1;
        } else {
            bottom[side] += 1;
        }
    }
    (top[0].max(top[1]), bottom[0].max(bottom[1]))
}

/// Physical strip pixels from the client height. A plain tag row
/// is ~1.6x the font plus pill padding; rows are spaced 4px with
/// an 8px safe-area inset — the same constants the tao/gtk
/// overlays lay out with, so a notification+status stack never
/// overflows the strip into game content.
pub fn strip_pixels(client_height: u32, rows: u32) -> u32 {
    if rows == 0 || client_height == 0 {
        return 0;
    }
    let font_px = (f64::from(client_height) * 0.022).ceil() as u32;
    let row = (font_px * 16).div_ceil(10) + 4;
    rows.saturating_mul(row) + 8 + 4 * rows.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::super::notification::NotificationKind;
    use super::*;
    use crate::session::commands::SlotOpFailure;
    use nerust_settings_core::model::language::AppLanguage;

    fn settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    #[test]
    fn error_modal_collects_and_skips_osd() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::SlotFailed {
                    slot: Some(1),
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        assert!(manager.rows().is_empty());
        assert_eq!(manager.take_modal().len(), 1);
    }

    #[test]
    fn info_expires_after_duration() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        assert!(manager.poll(now));
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.poll(now + Duration::from_millis(2_001)));
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn sequential_shows_one_then_next() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        for seq in 0..2 {
            manager.push_all(
                vec![UiNotification {
                    level: NotificationLevel::Info,
                    kind: if seq == 0 {
                        NotificationKind::Paused
                    } else {
                        NotificationKind::Resumed
                    },
                    count: 1,
                    seq,
                    at: std::time::SystemTime::now(),
                }],
                &settings(),
                AppLanguage::English,
                now,
            );
        }
        // Showing the first; the second waits its turn.
        assert!(manager.poll(now));
        assert_eq!(manager.rows().len(), 1);
        assert_eq!(manager.rows()[0].text, "Paused");
        // First expires after 2s; the second takes over.
        assert!(manager.poll(now + Duration::from_millis(2_001)));
        assert_eq!(manager.rows()[0].text, "Resumed");
        // Second expires; idle afterwards.
        assert!(manager.poll(now + Duration::from_millis(4_002)));
        assert!(manager.rows().is_empty());
        assert!(!manager.has_visible());
    }

    #[test]
    fn queue_caps_pending_and_coalesces_tail() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        // Five distinct pushes: showing + two queued, oldest dropped.
        for (seq, kind) in [
            NotificationKind::Paused,
            NotificationKind::Resumed,
            NotificationKind::Reset,
            NotificationKind::CreatedSlot,
            NotificationKind::Paused,
        ]
        .into_iter()
        .enumerate()
        {
            manager.push_all(
                vec![UiNotification {
                    level: NotificationLevel::Info,
                    kind,
                    count: 1,
                    seq: seq as u64,
                    at: std::time::SystemTime::now(),
                }],
                &settings(),
                AppLanguage::English,
                now,
            );
        }
        // Identical repeat merges with the tail (×2) instead of queuing.
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 99,
                at: std::time::SystemTime::now(),
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        assert!(manager.poll(now));
        // Oldest pending (CreatedSlot) shows first; drops removed older.
        assert!(manager.rows()[0].text.contains("Slot"));
        // Drain fully: CreatedSlot, then merged Paused ×2, then idle.
        let mut shown = vec![manager.rows()[0].text.clone()];
        let mut t = now;
        for _ in 0..5 {
            t += Duration::from_millis(2_001);
            if manager.poll(t) && !manager.rows().is_empty() {
                shown.push(manager.rows()[0].text.clone());
            }
        }
        assert!(manager.rows().is_empty());
        assert_eq!(shown.len(), 2);
        assert!(shown[1].contains("×2"));
    }

    #[test]
    fn error_osd_holds_five_second_minimum() {
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &altered,
            AppLanguage::English,
            now,
        );
        assert!(manager.poll(now));
        assert_eq!(manager.rows().len(), 1);
        // Still showing past the Info duration: 5s minimum.
        assert!(!manager.poll(now + Duration::from_millis(2_001)));
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.poll(now + Duration::from_millis(5_001)));
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn strip_rows_share_edges() {
        use NotificationPosition as P;
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                P::BottomRight,
                true
            ),
            (0, 1)
        );
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                P::BottomLeft,
                true
            ),
            (0, 2)
        );
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Overlay,
                P::BottomLeft,
                P::TopLeft,
                true
            ),
            (0, 0)
        );
    }

    #[test]
    fn level_tag_and_name_stay_distinct() {
        use super::super::notification::NotificationLevel as L;
        assert_eq!(super::level_tag(L::Warning), "[WARN]");
        assert_eq!(super::level_name(L::Warning), "Warning");
        assert_eq!(super::level_name(L::Error), "Error");
    }

    #[test]
    fn history_datetime_is_local_wall_clock() {
        // Only this test writes TZ; the rest assert TZ-free
        // structure, so parallel execution stays sound.
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
        use super::super::notification::{NotificationKind, UiNotification};
        // Injecting SystemTime directly is impossible here, so the
        // batch is checked structurally: one row per event, ×N suffix
        // preserved, level carried through.
        let events = [
            UiNotification {
                level: super::super::notification::NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            },
            UiNotification {
                level: super::super::notification::NotificationLevel::Error,
                kind: NotificationKind::CommandFailed("Save"),
                count: 3,
                seq: 1,
                at: std::time::UNIX_EPOCH,
            },
        ];
        let rows = history_lines(
            events.iter(),
            nerust_settings_core::model::language::AppLanguage::English,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].datetime, format_datetime(std::time::UNIX_EPOCH));
        assert_eq!(rows[0].text, "Paused");
        assert!(rows[1].text.ends_with(" ×3"));
        assert_eq!(rows[1].count, 3);
    }

    #[test]
    fn modal_queue_caps_and_drops_oldest() {
        let mut manager = OsdManager::new();
        for i in 0..10 {
            manager.queue_modal(format!("batch {i}"));
        }
        assert_eq!(manager.modal_pending(), 8);
        // Oldest two dropped; order preserved after that.
        assert_eq!(manager.take_modal_batch().as_deref(), Some("batch 2"));
        manager.modal_closed();
    }

    #[test]
    fn level_names_stay_distinct_per_surface() {
        use super::super::notification::NotificationLevel as L;
        assert_eq!(super::level_tag(L::Info), "[INFO]");
        assert_eq!(super::level_name(L::Info), "Info");
        assert_eq!(super::level_label(L::Info, AppLanguage::English), "Info");
        assert_eq!(
            super::level_label(L::Warning, AppLanguage::Japanese),
            "警告"
        );
        assert_eq!(super::level_label(L::Error, AppLanguage::English), "Error");
        assert_eq!(
            super::level_label(L::Error, AppLanguage::Japanese),
            "エラー"
        );
    }

    #[test]
    fn status_text_names_pause_state() {
        assert_eq!(
            OsdManager::status_text(60.0, true, AppLanguage::English),
            "Paused 60.0 fps"
        );
        assert_eq!(
            OsdManager::status_text(60.0, true, AppLanguage::Japanese),
            "一時停止中 60.0 fps"
        );
        assert_eq!(
            OsdManager::status_text(59.9, false, AppLanguage::English),
            "59.9 fps"
        );
    }

    #[test]
    fn identical_pushes_merge_and_reset_the_timer() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |count| UiNotification {
            level: L::Info,
            kind: NotificationKind::Paused,
            count,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all(vec![note(1)], &settings(), AppLanguage::English, now);
        manager.push_all(vec![note(1)], &settings(), AppLanguage::English, now);
        assert!(manager.poll(now));
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.rows()[0].text.ends_with(" ×2"));
        assert!(manager.has_visible());
        // Still showing before the reset deadline; gone after it.
        assert!(!manager.poll(now + std::time::Duration::from_secs(1)));
        assert!(manager.poll(now + std::time::Duration::from_secs(61)));
        assert!(!manager.has_visible());
    }

    #[test]
    fn next_deadline_distinguishes_idle_and_queued() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        assert_eq!(manager.next_deadline(), None);
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        // Promoted on the next poll; deadline lands on the duration.
        assert!(manager.poll(now));
        assert!(manager.next_deadline().is_some());
    }

    #[test]
    fn strip_rows_counts_edges_and_sides() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode as M, NotificationPosition as P,
        };
        assert_eq!(
            super::strip_rows(M::Overlay, P::TopLeft, P::BottomLeft, true),
            (0, 0)
        );
        assert_eq!(
            super::strip_rows(M::Strip, P::TopLeft, P::BottomLeft, true),
            (1, 1)
        );
        assert_eq!(
            super::strip_rows(M::Strip, P::TopRight, P::BottomRight, true),
            (1, 1)
        );
        // Same corner stacks two rows on one side.
        assert_eq!(
            super::strip_rows(M::Strip, P::TopLeft, P::TopLeft, true),
            (2, 0)
        );
        // Status off: transient row only.
        assert_eq!(
            super::strip_rows(M::Strip, P::BottomLeft, P::TopLeft, false),
            (0, 1)
        );
    }

    #[test]
    fn modal_queue_shows_one_at_a_time() {
        let mut manager = OsdManager::new();
        manager.queue_modal("first".to_string());
        manager.queue_modal("second".to_string());
        assert_eq!(manager.take_modal_batch().as_deref(), Some("first"));
        // Open dialog blocks the next batch.
        assert_eq!(manager.take_modal_batch(), None);
        assert_eq!(manager.modal_pending(), 1);
        manager.modal_closed();
        assert_eq!(manager.take_modal_batch().as_deref(), Some("second"));
        manager.modal_closed();
        assert_eq!(manager.take_modal_batch(), None);
    }

    #[test]
    fn strip_pixels_scale_with_client_height() {
        assert_eq!(strip_pixels(1000, 1), 48);
        assert_eq!(strip_pixels(1000, 2), 92);
        assert_eq!(strip_pixels(0, 1), 0);
        assert_eq!(strip_pixels(1000, 0), 0);
    }
}
