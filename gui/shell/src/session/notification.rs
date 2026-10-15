//! Transient user notifications: queue, coalescing, burst cap, and history.
//!
//! `SessionHandle::run_command` pushes one notification per executed or
//! failed command. Frontends drain the pending queue on their frame tick
//! and render at most [`MAX_PENDING`] labels; everything is also kept in
//! a bounded history ring and the logs.

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
            NotificationKind::Paused => {
                if ja {
                    "一時停止しました".to_string()
                } else {
                    "Paused".to_string()
                }
            }
            NotificationKind::Resumed => {
                if ja {
                    "再開しました".to_string()
                } else {
                    "Resumed".to_string()
                }
            }
            NotificationKind::Reset => {
                if ja {
                    "リセットしました".to_string()
                } else {
                    "Reset".to_string()
                }
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
            NotificationKind::CreatedSlot => {
                if ja {
                    "スロットを作成しました".to_string()
                } else {
                    "Slot created".to_string()
                }
            }
            NotificationKind::SelectedSlot(id) => {
                Self::slot_text(Some(*id), language, "Selected", "選択しました")
            }
            NotificationKind::SlotFailed { slot, reason } => {
                let what = match reason {
                    SlotOpFailure::Empty => {
                        if ja {
                            "空きスロットです"
                        } else {
                            "slot is empty"
                        }
                    }
                    SlotOpFailure::Missing => {
                        if ja {
                            "データがありません"
                        } else {
                            "no data"
                        }
                    }
                    SlotOpFailure::Storage => {
                        if ja {
                            "保存に失敗しました"
                        } else {
                            "storage failure"
                        }
                    }
                    SlotOpFailure::Corrupt => {
                        if ja {
                            "データが壊れています"
                        } else {
                            "data is corrupt"
                        }
                    }
                    SlotOpFailure::Incompatible => {
                        if ja {
                            "互換性がありません"
                        } else {
                            "incompatible data"
                        }
                    }
                    SlotOpFailure::Unavailable => {
                        if ja {
                            "利用できません"
                        } else {
                            "unavailable"
                        }
                    }
                };
                match (slot, ja) {
                    (Some(id), true) => format!("スロット{id}: {what}"),
                    (Some(id), false) => format!("Slot {id}: {what}"),
                    (None, _) => what.to_string(),
                }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiNotification {
    pub level: NotificationLevel,
    pub kind: NotificationKind,
    /// Coalesced repeat count (1 = shown once).
    pub count: u32,
    pub seq: u64,
}

#[derive(Debug, Default)]
pub struct NotificationCenter {
    pending: VecDeque<UiNotification>,
    history: VecDeque<UiNotification>,
    next_seq: u64,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one event. History and logs record every event; the pending
    /// queue coalesces identical neighbours (`×N`) and drops the oldest
    /// past [`MAX_PENDING`].
    pub fn push(&mut self, kind: NotificationKind) {
        let level = kind.level();
        let seq = self.next_seq;
        self.next_seq += 1;
        let event = UiNotification {
            level,
            kind: kind.clone(),
            count: 1,
            seq,
        };
        match level {
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
        });
    }

    pub fn drain(&mut self) -> Vec<UiNotification> {
        self.pending.drain(..).collect()
    }

    pub fn history(&self) -> impl Iterator<Item = &UiNotification> {
        self.history.iter()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

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
}
