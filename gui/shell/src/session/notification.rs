//! Transient notifications: queue, coalescing, burst cap, history.
//! Frontends drain per tick; all events also go to history and logs.

use std::collections::VecDeque;

use nerust_settings_core::model::language::AppLanguage;

use super::commands::{SessionCommand, SessionCommandOutcome, SlotOpFailure};

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
    SlotFailed {
        slot: Option<u64>,
        reason: SlotOpFailure,
    },
    CommandFailed(&'static str),
}

/// Pick the language side. Splitting this out (instead of inline
/// `if ja` per arm) keeps `text()` under the complexity budget.
fn bilingual(ja: bool, en: &str, ja_text: &str) -> String {
    bilingual_str(ja, en, ja_text).to_string()
}

fn bilingual_str<'a>(ja: bool, en: &'a str, ja_text: &'a str) -> &'a str {
    if ja { ja_text } else { en }
}

impl NotificationKind {
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
            NotificationKind::CommandFailed(_) => NotificationLevel::Error,
            _ => NotificationLevel::Info,
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
            NotificationKind::SlotFailed { slot, reason } => {
                Self::slot_failed_text(*slot, *reason, language)
            }
            NotificationKind::CommandFailed(command) => {
                if ja {
                    format!("{command} に失敗しました")
                } else {
                    format!("{command} failed")
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
    /// `SelectedSlot`: high-frequency nav echo would spam stderr, so
    /// selection feedback is screen-only (history still keeps it).
    pub fn push(&mut self, kind: NotificationKind) {
        let level = kind.level();
        let seq = self.next_seq;
        self.next_seq += 1;
        self.history_seq += 1;
        let event = UiNotification {
            level,
            kind: kind.clone(),
            count: 1,
            seq,
            at: std::time::SystemTime::now(),
        };
        match level {
            NotificationLevel::Info if matches!(kind, NotificationKind::SelectedSlot(_)) => {}
            NotificationLevel::Info => log::info!("notification: {}", kind_text(&event)),
            NotificationLevel::Warning => log::warn!("notification: {}", kind_text(&event)),
            NotificationLevel::Error => log::error!("notification: {}", kind_text(&event)),
        }
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(event);
        if let Some(back) = self.pending.back_mut()
            && back.level == level
            && back.kind == kind
        {
            back.count += 1;
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(UiNotification {
            level,
            kind,
            count: 1,
            seq,
            at: std::time::SystemTime::now(),
        });
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

/// Log module emitting `notification:` lines; history rows map 1:1
/// to those records so the module column greps the logs.
pub const LOG_MODULE: &str = module_path!();

fn kind_text(event: &UiNotification) -> String {
    // Logs stay English for grep stability; UI text is localized at render.
    event.kind.text(AppLanguage::English)
}

/// Map a command outcome to notifications. Success notifies Info;
/// slot failures map to Warning/Error by reason; transport errors
/// become a generic Error.
pub fn outcome_notifications(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Vec<NotificationKind> {
    if let Some(reason) = outcome.slot_failure {
        let slot = match command {
            SessionCommand::SaveSlot(id)
            | SessionCommand::LoadSlot(id)
            | SessionCommand::DeleteSlot(id)
            | SessionCommand::SelectActiveSlot(id) => Some(id),
            SessionCommand::SaveActiveSlotOrNew | SessionCommand::LoadActiveSlot => active_slot,
            _ => None,
        };
        return vec![NotificationKind::SlotFailed { slot, reason }];
    }
    if !outcome.executed {
        return Vec::new();
    }
    let kind = match command {
        SessionCommand::Pause | SessionCommand::TogglePause => NotificationKind::Paused,
        SessionCommand::Resume => NotificationKind::Resumed,
        SessionCommand::Reset => NotificationKind::Reset,
        SessionCommand::CreateSlot => NotificationKind::CreatedSlot,
        SessionCommand::SaveActiveSlotOrNew => {
            NotificationKind::SavedSlot(active_slot.unwrap_or(0))
        }
        SessionCommand::LoadActiveSlot => NotificationKind::LoadedSlot(active_slot.unwrap_or(0)),
        SessionCommand::SelectActiveSlot(id) => NotificationKind::SelectedSlot(id),
        SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot => {
            NotificationKind::SelectedSlot(active_slot.unwrap_or(0))
        }
        SessionCommand::SaveSlot(id) => NotificationKind::SavedSlot(id),
        SessionCommand::LoadSlot(id) => NotificationKind::LoadedSlot(id),
        SessionCommand::DeleteSlot(id) => NotificationKind::DeletedSlot(id),
    };
    // TogglePause resolving is state-dependent; Pause/Resume arms above
    // already cover the steady states, so no extra handling is needed.
    vec![kind]
}

/// Resolve TogglePause to the steady arm entered from `was_paused`.
/// Notifications name the entered state, not the invoked command.
pub fn resolve_toggle(command: SessionCommand, was_paused: bool) -> SessionCommand {
    match (command, was_paused) {
        (SessionCommand::TogglePause, false) => SessionCommand::Pause,
        (SessionCommand::TogglePause, true) => SessionCommand::Resume,
        (other, _) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn slot_failure_maps_to_warning_or_error() {
        let warn = outcome_notifications(
            SessionCommand::LoadSlot(2),
            &SessionCommandOutcome {
                executed: false,
                needs_redraw: false,
                slot_failure: Some(SlotOpFailure::Missing),
            },
            None,
        );
        assert_eq!(
            warn,
            vec![NotificationKind::SlotFailed {
                slot: Some(2),
                reason: SlotOpFailure::Missing,
            }]
        );
        assert_eq!(warn[0].level(), NotificationLevel::Warning);
        let err = outcome_notifications(
            SessionCommand::SaveSlot(1),
            &SessionCommandOutcome {
                executed: false,
                needs_redraw: false,
                slot_failure: Some(SlotOpFailure::Storage),
            },
            None,
        );
        assert_eq!(err[0].level(), NotificationLevel::Error);
    }

    #[test]
    fn texts_render_both_languages() {
        let kind = NotificationKind::SavedSlot(3);
        assert!(kind.text(AppLanguage::English).contains('3'));
        assert!(kind.text(AppLanguage::Japanese).contains("スロット3"));
    }

    #[test]
    fn texts_cover_every_variant_in_both_languages() {
        use crate::session::commands::SlotOpFailure;
        let variants = vec![
            NotificationKind::Paused,
            NotificationKind::Resumed,
            NotificationKind::Reset,
            NotificationKind::SavedSlot(1),
            NotificationKind::LoadedSlot(2),
            NotificationKind::DeletedSlot(3),
            NotificationKind::CreatedSlot,
            NotificationKind::SelectedSlot(4),
            NotificationKind::CommandFailed("Reset"),
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
    fn toggle_resolves_to_the_entered_state() {
        use super::resolve_toggle as resolve;
        use SessionCommand as C;
        assert_eq!(resolve(C::TogglePause, false), C::Pause);
        assert_eq!(resolve(C::TogglePause, true), C::Resume);
        assert_eq!(resolve(C::Resume, true), C::Resume);
        // The resolved arm drives the notification: toggling out of
        // pause reads Resumed, never Paused.
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        assert_eq!(
            outcome_notifications(resolve(C::TogglePause, true), &ok, None),
            [NotificationKind::Resumed]
        );
    }

    #[test]
    fn severity_orders_info_below_warning_below_error() {
        assert!(NotificationLevel::Info.severity() < NotificationLevel::Warning.severity());
        assert!(NotificationLevel::Warning.severity() < NotificationLevel::Error.severity());
    }

    #[test]
    fn outcome_success_maps_each_command() {
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        let cases = [
            (SessionCommand::Pause, NotificationKind::Paused),
            (SessionCommand::Resume, NotificationKind::Resumed),
            (SessionCommand::Reset, NotificationKind::Reset),
            (SessionCommand::CreateSlot, NotificationKind::CreatedSlot),
            (
                SessionCommand::SelectNextSlot,
                NotificationKind::SelectedSlot(0),
            ),
            (
                SessionCommand::SelectPreviousSlot,
                NotificationKind::SelectedSlot(0),
            ),
            (SessionCommand::SaveSlot(2), NotificationKind::SavedSlot(2)),
            (SessionCommand::LoadSlot(2), NotificationKind::LoadedSlot(2)),
            (
                SessionCommand::DeleteSlot(2),
                NotificationKind::DeletedSlot(2),
            ),
            (
                SessionCommand::LoadActiveSlot,
                NotificationKind::LoadedSlot(0),
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(outcome_notifications(command, &ok, Some(0)), [expected]);
        }
        // Unexecuted commands notify nothing.
        let skipped = SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: None,
        };
        assert!(outcome_notifications(SessionCommand::Pause, &skipped, Some(0)).is_empty());
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
}
