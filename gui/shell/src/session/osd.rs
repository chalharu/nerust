//! OSD display state shared by frontends: queue folding, expiry,
//! burst caps, strip math, and modal aggregation. Frontends render
//! `rows()` and honor `next_deadline()`; all policy lives here so
//! tao/gtk/android behave identically.

use std::time::{Duration, Instant};

use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationPosition, NotificationSettings,
};
use nerust_settings_core::model::language::AppLanguage;

use super::notification::{MAX_PENDING, NotificationLevel, UiNotification};

/// Glyph per level: shape (not color alone) distinguishes severity.
pub fn level_glyph(level: NotificationLevel) -> char {
    match level {
        NotificationLevel::Info => 'i',
        NotificationLevel::Warning => '!',
        NotificationLevel::Error => '×',
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

#[derive(Debug, Clone)]
pub struct OsdRow {
    pub text: String,
    pub rgb: (u8, u8, u8),
    pub glyph: char,
}

struct OsdItem {
    row: OsdRow,
    /// `None` holds indefinitely (Error-as-OSD without successor).
    deadline: Option<Instant>,
    /// Earliest replacement time (Error-as-OSD holds 5s minimum).
    replaceable_at: Instant,
}

pub struct OsdManager {
    items: Vec<OsdItem>,
    modal: Vec<String>,
    error_hold: Option<OsdItem>,
}

impl OsdManager {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            modal: Vec::new(),
            error_hold: None,
        }
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

    fn color(level: NotificationLevel, settings: &NotificationSettings) -> (u8, u8, u8) {
        match level {
            NotificationLevel::Info => settings.info_color.rgb(),
            NotificationLevel::Warning => settings.warning_color.rgb(),
            NotificationLevel::Error => settings.error_color.rgb(),
        }
    }

    /// Fold drained shell notifications into display state.
    /// Errors go modal (aggregated) or OSD-hold per settings.
    pub fn push_all(
        &mut self,
        notifications: Vec<UiNotification>,
        settings: &NotificationSettings,
        language: AppLanguage,
        now: Instant,
    ) {
        for n in notifications {
            if !Self::enabled(n.level, settings) {
                continue;
            }
            let mut text = n.kind.text(language);
            if n.count > 1 {
                text = format!("{text} ×{}", n.count);
            }
            let row = OsdRow {
                text,
                rgb: Self::color(n.level, settings),
                glyph: level_glyph(n.level),
            };
            if n.level == NotificationLevel::Error {
                if settings.error_modal {
                    self.modal.push(row.text);
                    continue;
                }
                // Error hold: minimum 5s, replaced by the next error
                // once mature. An immature hold keeps its slot.
                let mature = self
                    .error_hold
                    .as_ref()
                    .is_none_or(|hold| hold.replaceable_at <= now);
                if mature {
                    self.error_hold = Some(OsdItem {
                        row,
                        deadline: None,
                        replaceable_at: now + Duration::from_secs(5),
                    });
                } else {
                    self.push_transient(
                        OsdItem {
                            row,
                            deadline: Some(
                                now + Duration::from_millis(Self::duration_ms(
                                    NotificationLevel::Error,
                                    settings,
                                )),
                            ),
                            replaceable_at: now,
                        },
                        now,
                    );
                }
                continue;
            }
            self.push_transient(
                OsdItem {
                    row,
                    deadline: Some(
                        now + Duration::from_millis(Self::duration_ms(n.level, settings)),
                    ),
                    replaceable_at: now,
                },
                now,
            );
        }
    }

    fn push_transient(&mut self, item: OsdItem, now: Instant) {
        // Error hold yields to fresh transients once mature.
        if let Some(hold) = self.error_hold.take() {
            if hold.replaceable_at <= now {
                drop(hold);
            } else {
                self.error_hold = Some(hold);
            }
        }
        if self.items.len() >= MAX_PENDING {
            self.items.remove(0);
        }
        self.items.push(item);
    }

    /// Drop expired items. Returns true when the view changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items
            .retain(|item| item.deadline.is_none_or(|deadline| deadline > now));
        before != self.items.len()
    }

    pub fn rows(&self) -> Vec<OsdRow> {
        let mut rows: Vec<OsdRow> = self.items.iter().map(|item| item.row.clone()).collect();
        if let Some(hold) = self.error_hold.as_ref() {
            rows.push(hold.row.clone());
        }
        rows
    }

    pub fn take_modal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.modal)
    }

    pub fn has_visible(&self) -> bool {
        !self.items.is_empty() || self.error_hold.is_some()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.items.iter().filter_map(|item| item.deadline).min()
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

/// Physical strip pixels from the client height. Label rows scale
/// with the viewport (2.2%); padding is fixed.
pub fn strip_pixels(client_height: u32, rows: u32) -> u32 {
    if rows == 0 || client_height == 0 {
        return 0;
    }
    let font_px = (f64::from(client_height) * 0.022).ceil() as u32;
    rows.saturating_mul(font_px.saturating_add(8))
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
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.poll(now + Duration::from_millis(2_001)));
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn error_osd_holds_until_replaced() {
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
            }],
            &altered,
            AppLanguage::English,
            now,
        );
        assert_eq!(manager.rows().len(), 1);
        // No deadline: still showing after Info would have expired.
        assert!(!manager.poll(now + Duration::from_secs(30)));
        assert_eq!(manager.rows().len(), 1);
    }

    #[test]
    fn strip_rows_share_edges() {
        use NotificationPosition as P;
        assert_eq!(
            strip_rows(NotificationAreaMode::Strip, P::BottomLeft, P::BottomRight, true),
            (0, 1)
        );
        assert_eq!(
            strip_rows(NotificationAreaMode::Strip, P::BottomLeft, P::BottomLeft, true),
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
    fn strip_pixels_scale_with_client_height() {
        assert_eq!(strip_pixels(1000, 1), 30);
        assert_eq!(strip_pixels(0, 1), 0);
        assert_eq!(strip_pixels(1000, 0), 0);
    }
}
