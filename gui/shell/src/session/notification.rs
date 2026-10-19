//! Transient notifications: queue, coalescing, burst cap, history.
//! Frontends drain per tick; all events also go to history and logs.

use std::collections::VecDeque;

use nerust_settings_core::model::language::AppLanguage;

use crate::session::error::SlotOpFailure;

/// Maximum labels a frontend shows at once. Older entries drop off.
pub const MAX_PENDING: usize = 3;
/// Bounded review history (`pause` menu surfaces this).
pub const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

impl NotificationLevel {
    /// Severity rank: filters show this level and above, so Info
    /// shows everything (no Debug/Trace exists below it).
    pub fn severity(self) -> u8 {
        match self {
            NotificationLevel::Info => 0,
            NotificationLevel::Warning => 1,
            NotificationLevel::Error => 2,
        }
    }
}

/// Structured notification content. Frontends render via [`NotificationKind::text`];
/// no per-frontend message table is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    Paused,
    Resumed,
    Reset,
    SavedSlot(u64),
    LoadedSlot(u64),
    DeletedSlot(u64),
    CreatedSlot,
    SelectedSlot(u64),
    /// Persistence paths could not be resolved: the game runs, but
    /// saves have nowhere to go. Pushed once at setup time instead of
    /// failing every later flush.
    PersistenceUnavailable,
    SlotFailed {
        slot: Option<u64>,
        reason: SlotOpFailure,
    },
    /// Command and out-of-band frontend failures share one shape: a
    /// stable id plus an optional bounded detail. The modal queue is
    /// capped in count but not in bytes, so detail stays truncated.
    CommandFailure {
        origin: FailureOrigin,
        id: &'static str,
        detail: String,
    },
}

/// Where a failure surfaced. Filter key for the history grid; also
/// selects the localized origin prefix (see `text`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOrigin {
    Command,
    Picker,
    Restore,
    Bridge,
}

/// Longest kept detail suffix. A pathological multi-megabyte message
/// must not wedge the capped modal queue; history still records the
/// head of the message for grep.
pub const MAX_EXTERNAL_DETAIL: usize = 512;

/// Pick the language side. Splitting this out (instead of inline
/// `if ja` per arm) keeps `text()` under the complexity budget.
fn bilingual(ja: bool, en: &str, ja_text: &str) -> String {
    bilingual_str(ja, en, ja_text).to_string()
}

fn bilingual_str<'a>(ja: bool, en: &'a str, ja_text: &'a str) -> &'a str {
    if ja { ja_text } else { en }
}

/// Latest-wins replace group for selection echoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceGroup {
    SelectedSlot,
}

impl NotificationKind {
    /// Single truncation point for failure details. Both command
    /// and external paths funnel here so a pathological message
    /// cannot wedge the count-capped modal queue.
    pub fn command_failure(origin: FailureOrigin, id: &'static str, detail: String) -> Self {
        Self::CommandFailure {
            origin,
            id,
            detail: detail.chars().take(MAX_EXTERNAL_DETAIL).collect(),
        }
    }

    pub fn level(&self) -> NotificationLevel {
        match self {
            NotificationKind::SlotFailed { reason, .. } => match reason {
                SlotOpFailure::Storage | SlotOpFailure::Corrupt | SlotOpFailure::Incompatible => {
                    NotificationLevel::Error
                }
                SlotOpFailure::Empty | SlotOpFailure::Missing | SlotOpFailure::Unavailable => {
                    NotificationLevel::Warning
                }
            },
            NotificationKind::CommandFailure { .. } => NotificationLevel::Error,
            NotificationKind::PersistenceUnavailable => NotificationLevel::Warning,
            NotificationKind::Paused
            | NotificationKind::Resumed
            | NotificationKind::Reset
            | NotificationKind::SavedSlot(_)
            | NotificationKind::LoadedSlot(_)
            | NotificationKind::DeletedSlot(_)
            | NotificationKind::CreatedSlot
            | NotificationKind::SelectedSlot(_) => NotificationLevel::Info,
        }
    }

    /// Latest-wins group for OSD replacement, if any. Only
    /// selection echoes collapse; everything else queues.
    pub fn replace_group(&self) -> Option<ReplaceGroup> {
        match self {
            NotificationKind::SelectedSlot(_) => Some(ReplaceGroup::SelectedSlot),
            _ => None,
        }
    }

    /// Modal fallback without a composited frame: errors and
    /// warnings stay visible, info is dropped (nothing to show on).
    pub fn wants_modal(&self) -> bool {
        !matches!(self.level(), NotificationLevel::Info)
    }

    /// Channel policy: the log line for this kind, or `None` when the
    /// kind is screen-only (history still records every event). New
    /// kinds declare their channel here instead of editing `push`.
    /// Exhaustive: a new kind must choose log or screen-only here.
    pub fn log_level(&self) -> Option<NotificationLevel> {
        match self {
            NotificationKind::SelectedSlot(_) => None,
            NotificationKind::Paused
            | NotificationKind::Resumed
            | NotificationKind::Reset
            | NotificationKind::SavedSlot(_)
            | NotificationKind::LoadedSlot(_)
            | NotificationKind::DeletedSlot(_)
            | NotificationKind::CreatedSlot
            | NotificationKind::PersistenceUnavailable
            | NotificationKind::SlotFailed { .. }
            | NotificationKind::CommandFailure { .. } => Some(self.level()),
        }
    }

    fn slot_text(slot: Option<u64>, language: AppLanguage, en: &str, ja: &str) -> String {
        match (slot, language) {
            (Some(id), AppLanguage::Japanese) => format!("スロット{id}に{ja}"),
            (Some(id), _) => format!("{en} slot {id}"),
            (None, AppLanguage::Japanese) => ja.to_string(),
            (None, _) => en.to_string(),
        }
    }

    pub fn text(&self, language: AppLanguage) -> String {
        let ja = matches!(language, AppLanguage::Japanese);
        match self {
            NotificationKind::Paused => bilingual(ja, "Paused", "一時停止しました"),
            NotificationKind::Resumed => bilingual(ja, "Resumed", "再開しました"),
            NotificationKind::Reset => bilingual(ja, "Reset", "リセットしました"),
            NotificationKind::CreatedSlot => {
                bilingual(ja, "Slot created", "スロットを作成しました")
            }
            NotificationKind::SavedSlot(id) => {
                Self::slot_text(Some(*id), language, "Saved to", "保存しました")
            }
            NotificationKind::LoadedSlot(id) => {
                Self::slot_text(Some(*id), language, "Loaded", "読み込みました")
            }
            NotificationKind::DeletedSlot(id) => {
                Self::slot_text(Some(*id), language, "Deleted", "削除しました")
            }
            NotificationKind::SelectedSlot(id) => {
                Self::slot_text(Some(*id), language, "Selected", "選択しました")
            }
            NotificationKind::PersistenceUnavailable => {
                bilingual(ja, "Saves unavailable", "保存機能が利用できません")
            }
            NotificationKind::SlotFailed { slot, reason } => {
                Self::slot_failed_text(*slot, *reason, language)
            }
            NotificationKind::CommandFailure { origin, id, detail } => {
                let prefix = match (origin, ja) {
                    (FailureOrigin::Command, true) => {
                        return if detail.is_empty() {
                            format!("{id} に失敗しました")
                        } else {
                            format!("{id} に失敗しました: {detail}")
                        };
                    }
                    (FailureOrigin::Command, false) => {
                        return if detail.is_empty() {
                            format!("{id} failed")
                        } else {
                            format!("{id} failed: {detail}")
                        };
                    }
                    (FailureOrigin::Picker, true) => "選択",
                    (FailureOrigin::Picker, false) => "Picker",
                    (FailureOrigin::Restore, true) => "復元",
                    (FailureOrigin::Restore, false) => "Restore",
                    (FailureOrigin::Bridge, true) => "通知",
                    (FailureOrigin::Bridge, false) => "Bridge",
                };
                if detail.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{prefix}: {detail}")
                }
            }
        }
    }

    fn slot_failed_text(slot: Option<u64>, reason: SlotOpFailure, language: AppLanguage) -> String {
        let ja = matches!(language, AppLanguage::Japanese);
        let what = Self::failure_reason_text(reason, ja);
        match (slot, ja) {
            (Some(id), true) => format!("スロット{id}: {what}"),
            (Some(id), false) => format!("Slot {id}: {what}"),
            (None, _) => what.to_string(),
        }
    }

    fn failure_reason_text(reason: SlotOpFailure, ja: bool) -> &'static str {
        match reason {
            SlotOpFailure::Empty => bilingual_str(ja, "slot is empty", "空きスロットです"),
            SlotOpFailure::Missing => bilingual_str(ja, "no data", "データがありません"),
            SlotOpFailure::Storage => bilingual_str(ja, "storage failure", "保存に失敗しました"),
            SlotOpFailure::Corrupt => bilingual_str(ja, "data is corrupt", "データが壊れています"),
            SlotOpFailure::Incompatible => {
                bilingual_str(ja, "incompatible data", "互換性がありません")
            }
            SlotOpFailure::Unavailable => bilingual_str(ja, "unavailable", "利用できません"),
        }
    }
}

/// Clock port for deterministic tests. Production reads the real
/// system clock; tests inject a fixed timestamp via `push_at`.
pub trait Clock {
    fn now_system() -> std::time::SystemTime;
}

/// Production clock: the real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_system() -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiNotification {
    pub level: NotificationLevel,
    pub kind: NotificationKind,
    /// Coalesced repeat count (1 = shown once).
    pub count: u32,
    pub seq: u64,
    pub at: std::time::SystemTime,
}

#[derive(Debug, Default)]
pub struct NotificationCenter {
    pending: VecDeque<UiNotification>,
    history: VecDeque<UiNotification>,
    next_seq: u64,
    /// History version: advanced on every push and clear so hosts
    /// can skip grid rebuilds while nothing changed. The history
    /// sync runs on every loop pass when the viewer is open; without
    /// this gate each pass reformats all rows for nothing.
    history_seq: u64,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one event. History records every event; the pending queue
    /// coalesces identical neighbours (`×N`) and drops the oldest
    /// past [`MAX_PENDING`]. Logs record every event eagerly except
    /// screen-only kinds (see [`NotificationKind::log_level`]).
    pub fn push(&mut self, kind: NotificationKind) {
        self.push_at(kind, SystemClock::now_system());
    }

    /// Push with an explicit timestamp. Tests inject fixed times.
    pub fn push_at(&mut self, kind: NotificationKind, at: std::time::SystemTime) {
        let level = kind.level();
        let seq = self.next_seq;
        self.next_seq += 1;
        self.history_seq += 1;
        let event = UiNotification {
            level,
            kind,
            count: 1,
            seq,
            at,
        };
        // Channel policy lives on the kind: screen-only kinds skip
        // the log line (history still records every event). Gated so
        // steady-state pushes don't format when the level is off.
        if let Some(emit) = event.kind.log_level() {
            let text;
            match emit {
                NotificationLevel::Info => {
                    if log::log_enabled!(log::Level::Info) {
                        text = kind_text(&event);
                        log::info!("notification: {text}");
                    }
                }
                NotificationLevel::Warning => {
                    if log::log_enabled!(log::Level::Warn) {
                        text = kind_text(&event);
                        log::warn!("notification: {text}");
                    }
                }
                NotificationLevel::Error => {
                    if log::log_enabled!(log::Level::Error) {
                        text = kind_text(&event);
                        log::error!("notification: {text}");
                    }
                }
            }
        }
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(event.clone());
        if let Some(back) = self.pending.back_mut()
            && back.level == event.level
            && back.kind == event.kind
        {
            back.count += 1;
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(event);
    }

    pub fn drain(&mut self) -> Vec<UiNotification> {
        self.pending.drain(..).collect()
    }

    pub fn history(&self) -> impl Iterator<Item = &UiNotification> {
        self.history.iter()
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.history_seq += 1;
    }

    /// Current history version (see `history_seq`).
    pub fn history_seq(&self) -> u64 {
        self.history_seq
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Single text source for history rows and OSD labels: base
/// text plus the `×N` suffix for coalesced repeats.
pub fn render_text(kind: &NotificationKind, count: u32, language: AppLanguage) -> String {
    append_count(kind.text(language), count)
}

/// Suffix rule shared by [`render_text`] and OSD rows (which keep
/// the bare base as their coalescing key).
pub fn append_count(base: String, count: u32) -> String {
    if count > 1 {
        format!("{base} ×{count}")
    } else {
        base
    }
}

/// Log module emitting `notification:` lines; history rows map 1:1
/// to those records so the module column greps the logs.
/// Stable history module column. A literal, not `module_path!`: refactors
/// must not ripple into persisted history views or grep filters.
pub const LOG_MODULE: &str = "notifications";

fn kind_text(event: &UiNotification) -> String {
    // Logs stay English for grep stability; UI text is localized at render.
    event.kind.text(AppLanguage::English)
}

#[cfg(test)]
mod tests {
    use crate::session::error::SlotOpFailure;
    use crate::session::notification::{
        FailureOrigin, MAX_PENDING, NotificationCenter, NotificationKind, NotificationLevel,
        ReplaceGroup, render_text,
    };
    use nerust_settings_core::model::language::AppLanguage;

    #[test]
    fn log_level_marks_selection_screen_only() {
        assert_eq!(NotificationKind::SelectedSlot(3).log_level(), None);
        assert_eq!(
            NotificationKind::Paused.log_level(),
            Some(NotificationLevel::Info)
        );
        assert_eq!(
            NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Unavailable,
            }
            .log_level(),
            Some(NotificationLevel::Warning)
        );
        assert_eq!(
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Command,
                id: "Reset",
                detail: String::new()
            }
            .log_level(),
            Some(NotificationLevel::Error)
        );
        assert_eq!(
            NotificationKind::PersistenceUnavailable.log_level(),
            Some(NotificationLevel::Warning)
        );
    }

    #[test]
    fn coalesces_identical_neighbours_and_caps_pending() {
        let mut center = NotificationCenter::new();
        center.push(NotificationKind::Paused);
        center.push(NotificationKind::Paused);
        assert_eq!(center.pending_len(), 1);
        center.push(NotificationKind::Resumed);
        center.push(NotificationKind::Reset);
        center.push(NotificationKind::CreatedSlot);
        assert_eq!(center.pending_len(), MAX_PENDING);
        let drained = center.drain();
        assert_eq!(drained.len(), MAX_PENDING);
        // Oldest (coalesced Paused ×2) dropped off.
        assert_eq!(drained[0].kind, NotificationKind::Resumed);
        // History keeps every event.
        assert_eq!(center.history().count(), 5);
    }

    #[test]
    fn texts_render_both_languages() {
        let kind = NotificationKind::SavedSlot(3);
        assert!(kind.text(AppLanguage::English).contains('3'));
        assert!(kind.text(AppLanguage::Japanese).contains("スロット3"));
    }

    #[test]
    fn texts_cover_every_variant_in_both_languages() {
        let variants = vec![
            NotificationKind::Paused,
            NotificationKind::Resumed,
            NotificationKind::Reset,
            NotificationKind::SavedSlot(1),
            NotificationKind::LoadedSlot(2),
            NotificationKind::DeletedSlot(3),
            NotificationKind::CreatedSlot,
            NotificationKind::SelectedSlot(4),
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Command,
                id: "Reset",
                detail: String::new(),
            },
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Picker,
                id: "",
                detail: "boom".to_string(),
            },
            NotificationKind::PersistenceUnavailable,
        ];
        for kind in &variants {
            assert!(!kind.text(AppLanguage::English).is_empty());
            assert!(!kind.text(AppLanguage::Japanese).is_empty());
        }
        // Slot suffixes and failure reasons in both languages.
        for reason in [
            SlotOpFailure::Empty,
            SlotOpFailure::Missing,
            SlotOpFailure::Storage,
            SlotOpFailure::Corrupt,
            SlotOpFailure::Incompatible,
            SlotOpFailure::Unavailable,
        ] {
            for slot in [None, Some(7)] {
                let kind = NotificationKind::SlotFailed { slot, reason };
                assert!(!kind.text(AppLanguage::English).is_empty());
                assert!(!kind.text(AppLanguage::Japanese).is_empty());
            }
        }
        assert!(
            NotificationKind::SavedSlot(7)
                .text(AppLanguage::English)
                .contains('7')
        );
        assert!(
            NotificationKind::SlotFailed {
                slot: Some(7),
                reason: SlotOpFailure::Storage,
            }
            .text(AppLanguage::Japanese)
            .contains("スロット7")
        );
    }

    #[test]
    fn severity_orders_info_below_warning_below_error() {
        assert!(NotificationLevel::Info.severity() < NotificationLevel::Warning.severity());
        assert!(NotificationLevel::Warning.severity() < NotificationLevel::Error.severity());
    }

    #[test]
    fn history_seq_advances_on_push_and_clear_only() {
        let mut center = NotificationCenter::new();
        assert_eq!(center.history_seq(), 0);
        center.push(NotificationKind::Paused);
        center.push(NotificationKind::Paused);
        assert_eq!(center.history_seq(), 2);
        let _ = center.drain();
        assert_eq!(center.history_seq(), 2);
        center.clear_history();
        assert_eq!(center.history_seq(), 3);
    }

    #[test]
    fn replace_group_flags_only_selection() {
        assert_eq!(
            NotificationKind::SelectedSlot(2).replace_group(),
            Some(ReplaceGroup::SelectedSlot)
        );
        for kind in [
            NotificationKind::Paused,
            NotificationKind::SavedSlot(1),
            NotificationKind::PersistenceUnavailable,
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Command,
                id: "Reset",
                detail: String::new(),
            },
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Bridge,
                id: "",
                detail: "x".to_string(),
            },
        ] {
            assert_eq!(kind.replace_group(), None);
        }
    }

    #[test]
    fn wants_modal_keeps_errors_and_warnings() {
        assert!(
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Command,
                id: "Reset",
                detail: String::new()
            }
            .wants_modal()
        );
        assert!(NotificationKind::PersistenceUnavailable.wants_modal());
        assert!(
            NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Storage,
            }
            .wants_modal()
        );
        assert!(
            NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Missing,
            }
            .wants_modal()
        );
        assert!(!NotificationKind::Paused.wants_modal());
        assert!(!NotificationKind::SelectedSlot(1).wants_modal());
    }

    #[test]
    fn render_text_adds_count_suffix() {
        assert_eq!(
            render_text(&NotificationKind::Paused, 1, AppLanguage::English),
            "Paused"
        );
        assert_eq!(
            render_text(&NotificationKind::Paused, 3, AppLanguage::English),
            "Paused ×3"
        );
        assert!(
            render_text(&NotificationKind::SavedSlot(7), 2, AppLanguage::Japanese).contains("×2")
        );
    }

    #[test]
    fn push_shares_one_event_between_history_and_queue() {
        let mut center = NotificationCenter::new();
        center.push(NotificationKind::Paused);
        let drained = center.drain();
        let history: Vec<_> = center.history().collect();
        assert_eq!(drained.len(), 1);
        assert_eq!(history.len(), 1);
        // One construction: same identity on both sides.
        assert_eq!(drained[0].seq, history[0].seq);
        assert_eq!(drained[0].at, history[0].at);
        assert_eq!(drained[0].count, history[0].count);
    }
}
