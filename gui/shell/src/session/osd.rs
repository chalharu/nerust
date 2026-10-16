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

/// Level tag prefixing OSD labels and history rows: text (not
/// color or shape alone) carries severity.
pub fn level_tag(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "[INFO]",
        NotificationLevel::Warning => "[WARN]",
        NotificationLevel::Error => "[ERROR]",
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
    /// YYYY-MM-DD HH:MM:SS (UTC) of the push.
    pub datetime: String,
    pub level: NotificationLevel,
    pub module: &'static str,
    pub text: String,
    pub count: u32,
}

/// Full UTC datetime from a `SystemTime` without extra dependencies.
pub fn format_datetime(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let clock = secs % 86_400;
    // Howard Hinnant's civil-from-days; 719468 = days to 1970-01-01.
    let era = (days as i64 + 719_468) / 146_097;
    let doe = (days as i64 + 719_468) - era * 146_097;
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

pub fn format_time(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
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
    fn history_datetime_formats_utc_date() {
        let epoch = std::time::UNIX_EPOCH;
        assert_eq!(format_datetime(epoch), "1970-01-01 00:00:00");
        let known = epoch + std::time::Duration::from_secs(1_769_990_400);
        assert_eq!(format_datetime(known), "2026-02-02 00:00:00");
    }

    #[test]
    fn strip_pixels_scale_with_client_height() {
        assert_eq!(strip_pixels(1000, 1), 48);
        assert_eq!(strip_pixels(1000, 2), 92);
        assert_eq!(strip_pixels(0, 1), 0);
        assert_eq!(strip_pixels(1000, 0), 0);
    }
}
