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
    /// Persistence paths could not be resolved: the game runs, but saves have nowhere to go.
    /// Pushed once at setup time instead of failing every later flush.
    PersistenceUnavailable,
    SlotFailed {
        slot: Option<u64>,
        reason: SlotOpFailure,
    },
    /// Command and out-of-band frontend failures share one shape: a stable id plus detail.
    /// The modal queue is capped in count but not in bytes, so detail stays truncated.
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

/// Longest kept detail suffix: a pathological message must not wedge the capped modal queue.
pub const MAX_EXTERNAL_DETAIL: usize = 512;

/// Stable localization key for one notification shape.
/// The domain emits this plus args; only the renderer localizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKey {
    Paused,
    Resumed,
    Reset,
    SlotCreated,
    SlotSaved,
    SlotLoaded,
    SlotDeleted,
    SlotSelected,
    SavesUnavailable,
    SlotFailed,
    CommandFailed,
}

/// Domain payload for one notification: key plus raw args, no language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Stable key selecting the localized template.
    pub key: MessageKey,
    /// Slot carried for slot-bound messages.
    pub slot: Option<u64>,
    /// Failure reason for slot/command failures.
    pub reason: Option<SlotOpFailure>,
    /// Failure origin for command failures.
    pub origin: Option<FailureOrigin>,
    /// Stable command id for command failures.
    pub id: &'static str,
    /// Truncated detail for command failures.
    pub detail: String,
}

impl Message {
    /// Key-only message with no args.
    pub fn unit(key: MessageKey) -> Self {
        Self {
            key,
            slot: None,
            reason: None,
            origin: None,
            id: "",
            detail: String::new(),
        }
    }

    /// Slot-bound message carrying one slot id.
    pub fn slot(key: MessageKey, slot: Option<u64>) -> Self {
        Self {
            slot,
            ..Self::unit(key)
        }
    }

    /// 64-bit coalescing key over the stable fields: equal messages hash
    /// equal; collisions only over-coalesce.
    pub fn coalescing_key(&self) -> u64 {
        fn disc(key: MessageKey) -> u8 {
            match key {
                MessageKey::Paused => 0,
                MessageKey::Resumed => 1,
                MessageKey::Reset => 2,
                MessageKey::SlotCreated => 3,
                MessageKey::SlotSaved => 4,
                MessageKey::SlotLoaded => 5,
                MessageKey::SlotDeleted => 6,
                MessageKey::SlotSelected => 7,
                MessageKey::SavesUnavailable => 8,
                MessageKey::SlotFailed => 9,
                MessageKey::CommandFailed => 10,
            }
        }
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;
        let mut hash = OFFSET;
        let mut mix = |bytes: &[u8]| {
            for &byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        };
        mix(&[disc(self.key)]);
        mix(&self.slot.unwrap_or(u64::MAX).to_le_bytes());
        mix(&[self.reason.map_or(u8::MAX, |r| r as u8)]);
        mix(&[self.origin.map_or(u8::MAX, |o| o as u8)]);
        mix(self.id.as_bytes());
        mix(self.detail.as_bytes());
        hash
    }
}

/// Single localization adapter: the only place turning keys into text.
#[derive(Debug, Clone, Copy)]
pub struct NotificationRenderer {
    /// Target language for every rendered string.
    pub language: AppLanguage,
}

impl NotificationRenderer {
    /// Render one domain event in the adapter language.
    pub fn render(&self, kind: &NotificationKind) -> String {
        self.render_message(&kind.message())
    }

    /// Render one prebuilt domain message: the single format point shared
    /// by the row and the coalescing key.
    pub fn render_message(&self, message: &Message) -> String {
        render_message(message, self.language)
    }

    /// Render one stable level name in the adapter language.
    pub fn level_name(&self, level: NotificationLevel) -> &'static str {
        level_key(level, self.language)
    }
}

/// Stable level key; only the renderer resolves it per language.
fn level_key(level: NotificationLevel, language: AppLanguage) -> &'static str {
    use nerust_settings_core::i18n::{UiText, text as ui_text};
    match level {
        NotificationLevel::Info => ui_text(language, UiText::NotificationInfo),
        NotificationLevel::Warning => ui_text(language, UiText::NotificationWarning),
        NotificationLevel::Error => ui_text(language, UiText::Error),
    }
}

/// Template fill for one domain message in one language.
fn render_message(message: &Message, language: AppLanguage) -> String {
    match message.key {
        MessageKey::Paused => bilingual(language, "Paused", "一時停止しました"),
        MessageKey::Resumed => bilingual(language, "Resumed", "再開しました"),
        MessageKey::Reset => bilingual(language, "Reset", "リセットしました"),
        MessageKey::SlotCreated => bilingual(language, "Slot created", "スロットを作成しました"),
        MessageKey::SlotSaved => slot_text(message.slot, language, "Saved to", "保存しました"),
        MessageKey::SlotLoaded => slot_text(message.slot, language, "Loaded", "読み込みました"),
        MessageKey::SlotDeleted => slot_text(message.slot, language, "Deleted", "削除しました"),
        MessageKey::SlotSelected => slot_text(message.slot, language, "Selected", "選択しました"),
        MessageKey::SavesUnavailable => {
            bilingual(language, "Saves unavailable", "保存機能が利用できません")
        }
        MessageKey::SlotFailed => {
            let reason = message.reason.unwrap_or(SlotOpFailure::Unavailable);
            slot_failed_text(message.slot, reason, language)
        }
        MessageKey::CommandFailed => {
            let origin = message.origin.unwrap_or(FailureOrigin::Command);
            command_failed_text(origin, message.id, &message.detail, language)
        }
    }
}

fn command_failed_text(
    origin: FailureOrigin,
    id: &str,
    detail: &str,
    language: AppLanguage,
) -> String {
    let prefix = match (origin, language) {
        (FailureOrigin::Command, AppLanguage::Japanese) => {
            return if detail.is_empty() {
                format!("{id} に失敗しました")
            } else {
                format!("{id} に失敗しました: {detail}")
            };
        }
        (FailureOrigin::Command, _) => {
            return if detail.is_empty() {
                format!("{id} failed")
            } else {
                format!("{id} failed: {detail}")
            };
        }
        (FailureOrigin::Picker, AppLanguage::Japanese) => "選択",
        (FailureOrigin::Picker, _) => "Picker",
        (FailureOrigin::Restore, AppLanguage::Japanese) => "復元",
        (FailureOrigin::Restore, _) => "Restore",
        (FailureOrigin::Bridge, AppLanguage::Japanese) => "通知",
        (FailureOrigin::Bridge, _) => "Bridge",
    };
    if detail.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {detail}")
    }
}

fn slot_failed_text(slot: Option<u64>, reason: SlotOpFailure, language: AppLanguage) -> String {
    let what = failure_reason_text(reason, language);
    match (slot, language) {
        (Some(id), AppLanguage::Japanese) => format!("スロット{id}: {what}"),
        (Some(id), _) => format!("Slot {id}: {what}"),
        (None, _) => what.to_string(),
    }
}

fn failure_reason_text(reason: SlotOpFailure, language: AppLanguage) -> &'static str {
    match reason {
        SlotOpFailure::Empty => bilingual_str(language, "slot is empty", "空きスロットです"),
        SlotOpFailure::Missing => bilingual_str(language, "no data", "データがありません"),
        SlotOpFailure::Storage => bilingual_str(language, "storage failure", "保存に失敗しました"),
        SlotOpFailure::Corrupt => {
            bilingual_str(language, "data is corrupt", "データが壊れています")
        }
        SlotOpFailure::Incompatible => {
            bilingual_str(language, "incompatible data", "互換性がありません")
        }
        SlotOpFailure::Unavailable => bilingual_str(language, "unavailable", "利用できません"),
    }
}

/// Pick the language side. Splitting this out (instead of inline
/// `language` matches per arm) keeps `text()` under the complexity budget.
fn bilingual(language: AppLanguage, en: &str, ja_text: &str) -> String {
    bilingual_str(language, en, ja_text).to_string()
}

fn slot_text(slot: Option<u64>, language: AppLanguage, en: &str, ja: &str) -> String {
    match (slot, language) {
        (Some(id), AppLanguage::Japanese) => format!("スロット{id}に{ja}"),
        (Some(id), _) => format!("{en} slot {id}"),
        (None, AppLanguage::Japanese) => ja.to_string(),
        (None, _) => en.to_string(),
    }
}

fn bilingual_str<'a>(language: AppLanguage, en: &'a str, ja_text: &'a str) -> &'a str {
    match language {
        AppLanguage::Japanese => ja_text,
        _ => en,
    }
}

/// Latest-wins replace group for selection echoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceGroup {
    SelectedSlot,
}

impl NotificationKind {
    /// 64-bit coalescing key over the full content: equal kinds hash equal;
    /// collisions only over-coalesce.
    pub fn coalescing_key(&self) -> u64 {
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;
        let mut hash = OFFSET;
        let mut mix = |bytes: &[u8]| {
            for &byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        };
        match self {
            Self::Paused => mix(b"paused"),
            Self::Resumed => mix(b"resumed"),
            Self::Reset => mix(b"reset"),
            Self::SavedSlot(id) => {
                mix(b"saved");
                mix(&id.to_le_bytes());
            }
            Self::LoadedSlot(id) => {
                mix(b"loaded");
                mix(&id.to_le_bytes());
            }
            Self::DeletedSlot(id) => {
                mix(b"deleted");
                mix(&id.to_le_bytes());
            }
            Self::CreatedSlot => mix(b"created"),
            Self::SelectedSlot(id) => {
                mix(b"selected");
                mix(&id.to_le_bytes());
            }
            Self::PersistenceUnavailable => mix(b"unavailable"),
            Self::SlotFailed { slot, reason } => {
                mix(b"slotfailed");
                mix(&slot.unwrap_or(u64::MAX).to_le_bytes());
                mix(&[*reason as u8]);
            }
            Self::CommandFailure { origin, id, detail } => {
                mix(b"command");
                mix(&[*origin as u8]);
                mix(id.as_bytes());
                mix(detail.as_bytes());
            }
        }
        hash
    }

    /// Single truncation point for failure details; both paths funnel here.
    /// A pathological message cannot wedge the count-capped modal queue.
    pub fn command_failure(origin: FailureOrigin, id: &'static str, detail: String) -> Self {
        Self::CommandFailure {
            origin,
            id,
            detail: detail.chars().take(MAX_EXTERNAL_DETAIL).collect(),
        }
    }

    /// Typed failure entry: the source chain renders to text once, at this boundary.
    /// Truncation and shaping stay in [`command_failure`](Self::command_failure).
    pub fn command_failure_from<E: std::fmt::Display + ?Sized>(
        origin: FailureOrigin,
        id: &'static str,
        error: &E,
    ) -> Self {
        Self::command_failure(origin, id, error.to_string())
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

    /// Channel policy: the log line for this kind, or `None` when the kind is screen-only.
    /// New kinds declare their channel here instead of editing `push`.
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

    /// Domain payload for this event: stable key plus raw args, no language.
    pub fn message(&self) -> Message {
        match self {
            NotificationKind::Paused => Message::unit(MessageKey::Paused),
            NotificationKind::Resumed => Message::unit(MessageKey::Resumed),
            NotificationKind::Reset => Message::unit(MessageKey::Reset),
            NotificationKind::CreatedSlot => Message::unit(MessageKey::SlotCreated),
            NotificationKind::SavedSlot(id) => Message::slot(MessageKey::SlotSaved, Some(*id)),
            NotificationKind::LoadedSlot(id) => Message::slot(MessageKey::SlotLoaded, Some(*id)),
            NotificationKind::DeletedSlot(id) => Message::slot(MessageKey::SlotDeleted, Some(*id)),
            NotificationKind::SelectedSlot(id) => {
                Message::slot(MessageKey::SlotSelected, Some(*id))
            }
            NotificationKind::PersistenceUnavailable => Message::unit(MessageKey::SavesUnavailable),
            NotificationKind::SlotFailed { slot, reason } => Message {
                key: MessageKey::SlotFailed,
                slot: *slot,
                reason: Some(*reason),
                origin: None,
                id: "",
                detail: String::new(),
            },
            NotificationKind::CommandFailure { origin, id, detail } => Message {
                key: MessageKey::CommandFailed,
                slot: None,
                reason: None,
                origin: Some(*origin),
                id,
                detail: detail.clone(),
            },
        }
    }

    /// Localized text via the single renderer adapter.
    pub fn text(&self, language: AppLanguage) -> String {
        NotificationRenderer { language }.render(self)
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

/// Bounded event log: every event is recorded; version advances on push and clear.
#[derive(Debug, Default)]
pub struct EventLog {
    history: VecDeque<UiNotification>,
    next_seq: u64,
    /// History version: advanced on every push and clear.
    /// Hosts skip grid rebuilds while it is unchanged.
    history_seq: u64,
}

impl EventLog {
    /// Record one event, assigning its sequence number.
    pub fn record(&mut self, kind: NotificationKind, at: std::time::SystemTime) -> UiNotification {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.history_seq += 1;
        let event = UiNotification {
            level: kind.level(),
            kind,
            count: 1,
            seq,
            at,
        };
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(event.clone());
        event
    }

    /// Borrowed history for grid rendering.
    pub fn history(&self) -> impl Iterator<Item = &UiNotification> {
        self.history.iter()
    }

    /// Clear the log, advancing the version.
    pub fn clear(&mut self) {
        self.history.clear();
        self.history_seq += 1;
    }

    /// Current history version (see `history_seq`).
    pub fn history_seq(&self) -> u64 {
        self.history_seq
    }
}

/// Coalescing pending queue: identical neighbours merge, oldest drops past the cap.
#[derive(Debug, Default)]
pub struct PendingQueue {
    pending: VecDeque<UiNotification>,
}

impl PendingQueue {
    /// Offer one logged event; identical neighbours coalesce.
    /// Key comparison is O(1); kinds keep `PartialEq` for tests only.
    pub fn offer(&mut self, event: UiNotification) {
        if let Some(back) = self.pending.back_mut()
            && back.level == event.level
            && back.kind.coalescing_key() == event.kind.coalescing_key()
        {
            back.count += 1;
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(event);
    }

    /// Drain the queued events for one OSD pass.
    pub fn drain(&mut self) -> Vec<UiNotification> {
        self.pending.drain(..).collect()
    }

    /// Queued event count.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the queue holds no events.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Log sink: the single emission point for notification log lines.
#[derive(Debug, Default)]
pub struct LogSink;

impl LogSink {
    /// Emit the log line for one event; screen-only kinds stay silent.
    pub fn emit(&self, event: &UiNotification) {
        // Screen-only kinds skip the log line; steady-state pushes skip formatting when gated.
        if let Some(emit) = event.kind.log_level() {
            let text;
            match emit {
                NotificationLevel::Info => {
                    if log::log_enabled!(log::Level::Info) {
                        text = kind_text(event);
                        log::info!("notification: {text}");
                    }
                }
                NotificationLevel::Warning => {
                    if log::log_enabled!(log::Level::Warn) {
                        text = kind_text(event);
                        log::warn!("notification: {text}");
                    }
                }
                NotificationLevel::Error => {
                    if log::log_enabled!(log::Level::Error) {
                        text = kind_text(event);
                        log::error!("notification: {text}");
                    }
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct NotificationCenter {
    log: EventLog,
    queue: PendingQueue,
    sink: LogSink,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one event: history records every event; the pending queue coalesces neighbours.
    /// Screen-only kinds skip the log line (see [`NotificationKind::log_level`]).
    pub fn push(&mut self, kind: NotificationKind) {
        self.push_at(kind, SystemClock::now_system());
    }

    /// Push with an explicit timestamp. Tests inject fixed times.
    pub fn push_at(&mut self, kind: NotificationKind, at: std::time::SystemTime) {
        let event = self.log.record(kind, at);
        self.sink.emit(&event);
        self.queue.offer(event);
    }

    pub fn drain(&mut self) -> Vec<UiNotification> {
        self.queue.drain()
    }

    pub fn history(&self) -> impl Iterator<Item = &UiNotification> {
        self.log.history()
    }

    pub fn clear_history(&mut self) {
        self.log.clear();
    }

    /// Current history version (see `history_seq`).
    pub fn history_seq(&self) -> u64 {
        self.log.history_seq()
    }

    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }
}

/// Single text source for history rows and OSD labels: base
/// text plus the `×N` suffix for coalesced repeats.
pub fn render_text(kind: &NotificationKind, count: u32, language: AppLanguage) -> String {
    append_count(NotificationRenderer { language }.render(kind), count)
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

/// Log module emitting `notification:` lines; history rows map 1:1 to those records.
/// Stable history module column: a literal, not `module_path!`.
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
    fn command_failure_from_renders_typed_source_at_boundary() {
        use crate::session::SessionError;
        let error = SessionError::NoCore;
        let kind = NotificationKind::command_failure_from(FailureOrigin::Command, "Pause", &error);
        match &kind {
            NotificationKind::CommandFailure { origin, id, detail } => {
                assert_eq!(*origin, FailureOrigin::Command);
                assert_eq!(*id, "Pause");
                assert_eq!(detail, &error.to_string());
            }
            other => panic!("unexpected kind: {other:?}"),
        }
        assert!(!kind.text(AppLanguage::English).is_empty());
        assert!(!kind.text(AppLanguage::Japanese).is_empty());
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

#[cfg(test)]
mod message_key_tests {
    use crate::session::notification::*;

    #[test]
    fn equal_messages_hash_equal_and_differ_otherwise() {
        let a = NotificationKind::SavedSlot(1).message();
        let b = NotificationKind::SavedSlot(1).message();
        let c = NotificationKind::SavedSlot(2).message();
        let d = NotificationKind::SavedSlot(1);
        assert_eq!(a.coalescing_key(), b.coalescing_key());
        assert_ne!(a.coalescing_key(), c.coalescing_key());
        // Kind keys are stable per content within their own space.
        assert_eq!(d.coalescing_key(), d.coalescing_key());
        // Every variant produces a stable key across calls.
        for kind in [
            NotificationKind::Paused,
            NotificationKind::CommandFailure {
                origin: FailureOrigin::Bridge,
                id: "x",
                detail: "boom".to_string(),
            },
            NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Corrupt,
            },
        ] {
            assert_eq!(kind.coalescing_key(), kind.coalescing_key());
        }
    }

    #[test]
    fn render_message_matches_render() {
        let renderer = NotificationRenderer {
            language: AppLanguage::English,
        };
        for kind in [
            NotificationKind::Paused,
            NotificationKind::SavedSlot(3),
            NotificationKind::SlotFailed {
                slot: Some(1),
                reason: SlotOpFailure::Storage,
            },
        ] {
            let message = kind.message();
            assert_eq!(renderer.render(&kind), renderer.render_message(&message));
        }
    }
}

#[cfg(test)]
mod key_tests {
    use crate::session::notification::*;

    #[test]
    fn queue_keys_on_message_not_text() {
        let a = NotificationKind::SavedSlot(1).message();
        let b = NotificationKind::SavedSlot(1).message();
        let c = NotificationKind::SavedSlot(2).message();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.key, MessageKey::SlotSaved);
        assert_eq!(a.slot, Some(1));
    }

    #[test]
    fn renderer_is_the_single_format_point() {
        let renderer = NotificationRenderer {
            language: AppLanguage::English,
        };
        assert_eq!(renderer.render(&NotificationKind::Paused), "Paused");
        assert!(!renderer.render(&NotificationKind::SavedSlot(7)).is_empty());
    }
}

#[cfg(test)]
mod notification_cov_tests {
    use crate::session::notification::*;

    #[test]
    fn pending_queue_reports_emptiness() {
        let mut queue = PendingQueue::default();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        queue.offer(UiNotification {
            level: NotificationLevel::Info,
            kind: NotificationKind::Paused,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        });
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.drain().len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn failure_detail_truncates_at_cap() {
        let long = "y".repeat(MAX_EXTERNAL_DETAIL + 50);
        let kind = NotificationKind::command_failure(FailureOrigin::Bridge, "id", long);
        match kind {
            NotificationKind::CommandFailure { detail, .. } => {
                assert_eq!(detail.chars().count(), MAX_EXTERNAL_DETAIL);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn message_slot_none_and_level_names() {
        let message = Message::slot(MessageKey::SlotSaved, None);
        assert_eq!(message.slot, None);
        assert_eq!(message.key, MessageKey::SlotSaved);
        let renderer = NotificationRenderer {
            language: AppLanguage::Japanese,
        };
        assert!(!renderer.level_name(NotificationLevel::Warning).is_empty());
        assert_eq!(append_count("a".to_string(), 1), "a");
        let _ = SystemClock::now_system();
        let mut log = EventLog::default();
        let _ = log.record(NotificationKind::Paused, std::time::UNIX_EPOCH);
        assert_eq!(log.history_seq(), 1);
        log.clear();
        assert_eq!(log.history_seq(), 2);
        assert_eq!(log.history().count(), 0);
    }
}
