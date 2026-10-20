//! OSD display state shared by frontends: folding, expiry, burst caps, and modal aggregation.
//! Frontends render `rows()` and honor `wakeup_deadline()`; all policy lives in sibling modules.

use std::time::Instant;

use nerust_settings_core::model::language::AppLanguage;

use super::notification::UiNotification;

pub use super::osd_history::{
    HistoryLine, format_datetime, format_datetime_with_offset, format_time, history_lines,
    level_label, level_name, level_tag,
};

pub use crate::session::osd_modal::{ModalSink, ModalSource};
pub use crate::session::osd_presenter::{
    OsdChrome, OsdPlan, OsdPresenter, OsdState, OsdSync, OsdSyncInput, STATUS_FPS_MIN_INTERVAL,
    StatusGate, TickMetrics, UiInsets, format_status_value,
};
pub use crate::session::osd_render::{OSD_STATUS_RGB, osd_anchor};
pub use crate::session::osd_transient::{FrameChannel, OsdRow, TransientPolicy};

use crate::session::osd_modal::{ModalCenter, ModalPump};
use crate::session::osd_presenter::{ModalRoute, ModalRouter, status_key, status_text};
use crate::session::osd_transient::{OsdItem, RouteEffect, TransientQueue, counted_row};

/// Notification display state: transient in-frame rows plus modal batches.
#[derive(Debug, Default)]
pub struct OsdManager {
    transient: TransientQueue,
    modal: ModalCenter,
    /// Monotonic display-generation. Presenters remember the last
    /// generation they synced; inequality means re-render.
    generation: u64,
}

/// Bundled inputs for [`SessionHandle::pump_osd`]: the caller-owned half of the push.
/// The tick is fetched once per pass and shared with the sync decision.
pub struct PushInput {
    pub policy: TransientPolicy,
    pub language: AppLanguage,
    pub now: Instant,
    pub tick: TickMetrics,
}

/// Outcome of [`SessionHandle::pump_osd`] for frontend flow control:
/// redraw gating and modal auto-pause.
pub struct PumpReport {
    pub changed: bool,
    pub modal_queued: bool,
}

impl OsdManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push taking the narrow policy; channel-to-modal routing lives in [`ModalRouter`].
    /// Callers adapt once at the settings boundary via `From`.
    pub fn push_all_with_policy(
        &mut self,
        notifications: impl IntoIterator<Item = UiNotification>,
        policy: &TransientPolicy,
        language: AppLanguage,
        now: Instant,
        channel: FrameChannel,
    ) {
        // Dirty only on a real change: filtered rows, dropped info,
        // and queued-behind rows must not schedule a redraw.
        let mut changed = false;
        let router = ModalRouter { policy, channel };
        for n in notifications {
            match router.route(&n.kind) {
                // No composited frame and not modal-worthy: drop.
                // Disabled levels never display.
                ModalRoute::Drop => continue,
                ModalRoute::Modal => {
                    self.modal
                        .push_text(super::notification::render_text(&n.kind, n.count, language));
                    changed = true;
                    continue;
                }
                ModalRoute::Transient => {}
            }
            let renderer = super::notification::NotificationRenderer { language };
            let base: std::rc::Rc<str> = renderer.render(&n.kind).into();
            let item = OsdItem {
                row: counted_row(&base, n.count, policy.color(n.level), level_tag(n.level)),
                level: n.level,
                message: n.kind.message(),
                base,
                count: n.count,
                duration: policy.duration(n.level),
                deadline: now + policy.duration(n.level),
                group: n.kind.replace_group(),
            };
            if self.transient.route_row(item) == RouteEffect::ShowingChanged {
                changed = true;
            }
        }
        if changed {
            self.mark_changed();
        }
    }

    /// Advance the showing label. Signals via `dirty()`, which also
    /// covers `push_all` (a bool return would miss those changes).
    pub fn poll(&mut self, now: Instant) {
        if self.transient.poll(now) {
            self.mark_changed();
        }
    }

    pub fn rows(&self) -> &[OsdRow] {
        self.transient.rows()
    }

    /// Borrowed showing row. The sync emptiness check uses this so the
    /// clear path clones nothing; only `Render` builds the owned plan.
    pub fn visible_row(&self) -> Option<&OsdRow> {
        self.transient.visible_row()
    }

    /// Drain the aggregated modal batch into the sequential queue.
    /// Bullet formatting happens here; overflow warns once per episode until the queue drains.
    pub(crate) fn pump_modal(&mut self) -> bool {
        let lines = self.modal.take_batch_text();
        if lines.is_empty() {
            return false;
        }
        // Single source for the aggregation wording: dialogs show it
        // as-is on every frontend, Android included.
        let detail = lines
            .iter()
            .map(|line| format!("• {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        match self.modal.pump(Some(detail)) {
            ModalPump::Idle => false,
            ModalPump::Queued { dropped } => {
                if dropped && !self.modal.overflow_already_reported() {
                    log::warn!("error modal queue full; dropping oldest batch");
                    self.modal.note_overflow_reported();
                }
                true
            }
        }
    }

    #[cfg(test)]
    fn take_modal_text(&mut self) -> Vec<String> {
        self.modal.take_batch_text()
    }

    /// Monotonic change count. `dirty`/`take_dirty` stay for the
    /// peek-then-take protocol; new code can poll `generation`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn mark_changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

impl ModalSink for OsdManager {
    fn queue_modal(&mut self, detail: String) {
        self.modal.queue_direct(detail);
    }

    fn requeue_front(&mut self, detail: String) {
        self.modal.requeue_direct(detail);
    }
}

impl ModalSource for OsdManager {
    fn take_modal_batch(&mut self) -> Option<String> {
        self.modal.take_batch()
    }

    fn modal_closed(&mut self) {
        self.modal.closed()
    }

    fn modal_pending(&self) -> usize {
        self.modal.pending()
    }
}

/// Narrow pump surface for [`SessionHandle::pump_osd`], decoupled from the orchestration type.
pub trait PumpTarget: OsdState + ModalSink {
    /// Route one drained batch through the transient/modal policy.
    fn push_notifications(
        &mut self,
        notifications: Vec<UiNotification>,
        policy: &TransientPolicy,
        language: AppLanguage,
        now: Instant,
        channel: FrameChannel,
    );
    /// Advance the showing label.
    fn poll_queue(&mut self, now: Instant);
    /// Drain the aggregated modal batch into the sequential queue.
    /// Returns whether a batch was queued.
    fn pump_modal_queue(&mut self) -> bool;
}

impl PumpTarget for OsdManager {
    fn push_notifications(
        &mut self,
        notifications: Vec<UiNotification>,
        policy: &TransientPolicy,
        language: AppLanguage,
        now: Instant,
        channel: FrameChannel,
    ) {
        self.push_all_with_policy(notifications, policy, language, now, channel);
    }

    fn poll_queue(&mut self, now: Instant) {
        self.poll(now);
    }

    fn pump_modal_queue(&mut self) -> bool {
        self.pump_modal()
    }
}

impl OsdManager {
    /// Total modal overflow losses (test hook for the warn-once path).
    #[cfg(test)]
    pub fn overflow_dropped(&self) -> u64 {
        self.modal.dropped_total()
    }

    /// Queued error batches awaiting a dialog, for the wake policy.
    pub fn modal_pending(&self) -> usize {
        self.modal.pending()
    }

    pub fn has_visible(&self) -> bool {
        self.transient.has_visible()
    }

    pub fn wakeup_deadline(&self, now: Instant) -> Option<Instant> {
        self.transient.wakeup_deadline(now)
    }

    /// Status readout split into label and value for the fixed-width pill.
    /// Digits never shift when the label appears, disappears, or changes language.
    pub fn status_parts(fps: f32, paused: bool, language: AppLanguage) -> (String, String) {
        (
            status_text(status_key(paused), language).to_string(),
            super::osd_presenter::format_status_value(fps),
        )
    }
}

impl crate::session::osd_presenter::OsdState for OsdManager {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn visible_row(&self) -> Option<&OsdRow> {
        self.visible_row()
    }

    fn rows(&self) -> &[OsdRow] {
        self.rows()
    }
}

#[cfg(test)]
mod tests {
    use super::super::notification::NotificationKind;
    use super::*;
    use crate::session::error::SlotOpFailure;
    use crate::session::notification::NotificationLevel;
    use nerust_settings_core::model::language::AppLanguage;
    use nerust_settings_core::model::local::NotificationSettings;
    use std::time::Duration;

    fn settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    fn policy() -> TransientPolicy {
        TransientPolicy::from(&settings())
    }

    #[test]
    fn generation_advances_only_on_visible_change() {
        use super::super::notification::NotificationKind as K;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: K::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        // Unpresented info drops: no visible change, no generation bump.
        assert_eq!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: K::CommandFailure {
                    origin: super::super::notification::FailureOrigin::Bridge,
                    id: "",
                    detail: "boom".to_string(),
                },
                count: 1,
                seq: 1,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert_eq!(manager.generation(), base.wrapping_add(1));
    }

    #[test]
    fn external_error_routes_modal_without_frame() {
        use super::super::notification::{FailureOrigin, NotificationKind as K};
        let kind = K::CommandFailure {
            origin: FailureOrigin::Picker,
            id: "",
            detail: "picker failed".to_string(),
        };
        assert_eq!(kind.level(), NotificationLevel::Error);
        assert!(kind.wants_modal());
        assert_eq!(kind.text(AppLanguage::English), "Picker: picker failed");
        assert_eq!(kind.text(AppLanguage::Japanese), "選択: picker failed");
        let mut manager = OsdManager::new();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: kind.level(),
                kind,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            Instant::now(),
            FrameChannel::Unpresented,
        );
        assert!(manager.rows().is_empty());
        assert!(!manager.take_modal_text().is_empty());
    }

    #[test]
    fn error_modal_collects_and_skips_osd() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
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
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert!(manager.rows().is_empty());
        assert!(!manager.take_modal_text().is_empty());
    }

    #[test]
    fn inactive_core_routes_errors_and_warnings_to_modal() {
        let mut manager = OsdManager::new();
        let mut without_modal = settings();
        without_modal.error_modal = false;
        let now = Instant::now();
        for (level, kind) in [
            (
                NotificationLevel::Error,
                NotificationKind::SlotFailed {
                    slot: Some(1),
                    reason: SlotOpFailure::Storage,
                },
            ),
            (
                NotificationLevel::Warning,
                NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
            ),
            (NotificationLevel::Info, NotificationKind::Paused),
        ] {
            manager.push_all_with_policy(
                vec![UiNotification {
                    level,
                    kind,
                    count: 1,
                    seq: 0,
                    at: std::time::SystemTime::now(),
                }],
                &TransientPolicy::from(&without_modal),
                AppLanguage::English,
                now,
                FrameChannel::Unpresented,
            );
        }
        assert!(manager.rows().is_empty());
        // Info is dropped without a frame: only two modal lines.
        let batch = manager.take_modal_text();
        assert_eq!(batch.len(), 2);
        assert!(batch.iter().all(|line| !line.contains("Paused")));
    }

    #[test]
    fn info_dropped_without_frame_leaves_no_trace() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert_eq!(manager.generation(), base);
        assert!(manager.rows().is_empty());
        assert!(manager.take_modal_text().is_empty());
        assert_eq!(manager.modal_pending(), 0);
    }

    #[test]
    fn info_expires_after_duration() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn sequential_shows_one_then_next() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        for seq in 0..2 {
            manager.push_all_with_policy(
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
                &policy(),
                AppLanguage::English,
                now,
                FrameChannel::Composited,
            );
        }
        // Showing the first; the second waits its turn.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        assert_eq!(manager.rows()[0].text, "Paused");
        // First expires after 2s; the second takes over.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].text, "Resumed");
        // Second expires; idle afterwards.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(4_002));
        assert_ne!(manager.generation(), base);
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
            manager.push_all_with_policy(
                vec![UiNotification {
                    level: NotificationLevel::Info,
                    kind,
                    count: 1,
                    seq: seq as u64,
                    at: std::time::SystemTime::now(),
                }],
                &policy(),
                AppLanguage::English,
                now,
                FrameChannel::Composited,
            );
        }
        // Identical repeat merges with the tail (×2) instead of queuing.
        manager.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 99,
                at: std::time::SystemTime::now(),
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Oldest pending (CreatedSlot) shows first; drops removed older.
        assert!(manager.rows()[0].text.contains("Slot"));
        // Drain fully: CreatedSlot, then merged Paused ×2, then idle.
        let mut shown = vec![manager.rows()[0].text.clone()];
        let mut t = now;
        for _ in 0..5 {
            t += Duration::from_millis(2_001);
            let base = manager.generation();
            manager.poll(t);
            if manager.generation() != base && !manager.rows().is_empty() {
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
        manager.push_all_with_policy(
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
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        // Still showing past the Info duration: 5s minimum.
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_eq!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn error_survives_info_flood() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |kind, level| UiNotification {
            level,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let error = || UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Showing Info; two Infos queued (full).
        manager.push_all_with_policy(
            vec![note(NotificationKind::Paused, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![note(NotificationKind::Resumed, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(NotificationKind::Reset, L::Info)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // An Error preempts the showing Info; the demoted label waits
        // at the queue head while the tail sheds the newest Info.
        manager.push_all_with_policy(
            vec![error()],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Paused"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(7_002));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Resumed"));
    }

    #[test]
    fn error_preempts_showing_info() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[INFO]");
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Error,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // The Error takes the display at once; the demoted Info waits.
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[INFO]");
    }

    #[test]
    fn warning_waits_behind_showing_info() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Warning,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // Only Errors preempt; the Warning waits for expiry.
        assert_eq!(manager.rows()[0].tag, "[INFO]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
    }

    #[test]
    fn queue_overflow_drops_lowest_severity_first() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let warning = UiNotification {
            level: L::Warning,
            kind: NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Missing,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let info = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Showing Error (no preempt against itself); queue Warning + Info.
        manager.push_all_with_policy(
            vec![error(Some(9))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![warning],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![info(NotificationKind::Paused)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // A second Info overflows: the oldest Info drops, Warning and
        // Error survive.
        manager.push_all_with_policy(
            vec![info(NotificationKind::Reset)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(10_002));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Reset"));
        assert_eq!(manager.rows()[0].tag, "[INFO]");
    }

    #[test]
    fn error_queue_bound_drops_oldest_error() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Three Errors, nothing showing: the oldest drops (FIFO bound).
        manager.push_all_with_policy(
            vec![error(Some(1))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![error(Some(2))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![error(Some(3))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Slot 2"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Slot 3"));
    }

    #[test]
    fn demote_overflow_sheds_tail_info_first() {
        use super::super::notification::NotificationLevel as L;
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let error = |slot| UiNotification {
            level: L::Error,
            kind: NotificationKind::SlotFailed {
                slot,
                reason: SlotOpFailure::Storage,
            },
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let info = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        // Errors preempt a showing Info, so a full Error queue here is unreachable.
        // The demote victim in this shape is the tail Info.
        manager.push_all_with_policy(
            vec![info(NotificationKind::Paused)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Warning,
                kind: NotificationKind::SlotFailed {
                    slot: None,
                    reason: SlotOpFailure::Missing,
                },
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![info(NotificationKind::Reset)],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // The Error preempts; the demote insert overflows and the
        // tail Info drops so the demoted head survives.
        manager.push_all_with_policy(
            vec![error(Some(1))],
            &TransientPolicy::from(&altered),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.rows()[0].tag, "[ERROR]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(5_000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("Paused"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(7_000));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].tag, "[WARN]");
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(12_000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn selected_slot_latest_wins() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |slot| UiNotification {
            level: L::Info,
            kind: NotificationKind::SelectedSlot(slot),
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(3)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(5)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // One row only, showing the latest selection.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("slot 5"));
        assert!(!manager.rows()[0].text.contains("slot 3"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn selected_slot_updates_showing_row() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |slot| UiNotification {
            level: L::Info,
            kind: NotificationKind::SelectedSlot(slot),
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(3)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.contains("slot 3"));
        // A newer selection rewrites the showing row instead of queuing.
        manager.push_all_with_policy(
            vec![note(7)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert!(manager.rows()[0].text.contains("slot 7"));
        let base = manager.generation();
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
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
    fn status_text_names_pause_state() {
        assert_eq!(
            OsdManager::status_parts(60.0, true, AppLanguage::English),
            ("Paused".to_string(), "60.0 fps".to_string())
        );
        assert_eq!(
            OsdManager::status_parts(60.0, true, AppLanguage::Japanese),
            ("一時停止中".to_string(), "60.0 fps".to_string())
        );
        assert_eq!(
            OsdManager::status_parts(59.9, false, AppLanguage::English),
            (String::new(), "59.9 fps".to_string())
        );
    }

    #[test]
    fn interleaved_pause_resume_queues_sequentially() {
        use super::super::notification::NotificationKind as K;
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let note = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        macro_rules! shown {
            () => {
                manager.rows().first().map(|row| row.text.clone())
            };
        }
        // Pause, +0.5s resume, +0.5s pause, +0.5s resume (2s slots).
        manager.push_all_with_policy(
            vec![note(K::Paused)],
            &policy(),
            AppLanguage::English,
            at(0),
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(at(0));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Paused"));
        manager.push_all_with_policy(
            vec![note(K::Resumed)],
            &policy(),
            AppLanguage::English,
            at(500),
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(at(500));
        assert_eq!(manager.generation(), base);
        // Interleaved: must queue behind, never merge into the showing label.
        manager.push_all_with_policy(
            vec![note(K::Paused)],
            &policy(),
            AppLanguage::English,
            at(1000),
            FrameChannel::Composited,
        );
        assert_eq!(shown!().as_deref(), Some("Paused"));
        manager.push_all_with_policy(
            vec![note(K::Resumed)],
            &policy(),
            AppLanguage::English,
            at(1500),
            FrameChannel::Composited,
        );
        // Full 2s slots in arrival order; promoted labels measure from promotion, not push time.
        // The cap drops the oldest pending, so the legs read pause, pause, resume.
        let base = manager.generation();
        manager.poll(at(2000));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Paused"));
        let base = manager.generation();
        manager.poll(at(3500));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(at(4000));
        assert_ne!(manager.generation(), base);
        assert_eq!(shown!().as_deref(), Some("Resumed"));
        let base = manager.generation();
        manager.poll(at(5500));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(at(6000));
        assert_ne!(manager.generation(), base);
        assert!(manager.rows().is_empty());
        let base = manager.generation();
        manager.poll(at(8000));
        assert_eq!(manager.generation(), base);
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
        manager.push_all_with_policy(
            vec![note(1)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.push_all_with_policy(
            vec![note(1)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows().len(), 1);
        assert!(manager.rows()[0].text.ends_with(" ×2"));
        assert!(manager.has_visible());
        // Still showing before the reset deadline; gone after it.
        let base = manager.generation();
        manager.poll(now + std::time::Duration::from_secs(1));
        assert_eq!(manager.generation(), base);
        let base = manager.generation();
        manager.poll(now + std::time::Duration::from_secs(61));
        assert_ne!(manager.generation(), base);
        assert!(!manager.has_visible());
    }

    #[test]
    fn wakeup_deadline_distinguishes_idle_and_queued() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        assert_eq!(manager.wakeup_deadline(now), None);
        manager.push_all_with_policy(
            vec![UiNotification {
                level: L::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        // Pending with nothing showing: wake immediately on our clock.
        assert_eq!(manager.wakeup_deadline(now), Some(now));
        // Promoted on the next poll; deadline lands on the duration.
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        assert!(manager.wakeup_deadline(now).is_some());
    }

    #[test]
    fn queued_behind_leaves_clean_until_promotion() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = |kind| UiNotification {
            level: L::Info,
            kind,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note(NotificationKind::Paused)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Queued behind the showing row: no dirty, deadline armed.
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![note(NotificationKind::Resumed)],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(manager.generation(), base);
        assert!(manager.wakeup_deadline(now).is_some());
        // Expiry promotes the queued row and dirties.
        manager.poll(now + Duration::from_millis(2_001));
        assert_ne!(manager.generation(), base);
        assert_eq!(manager.rows()[0].text, "Resumed");
    }

    #[test]
    fn merged_showing_row_dirties() {
        use super::super::notification::NotificationLevel as L;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        let note = UiNotification {
            level: L::Info,
            kind: NotificationKind::Paused,
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        manager.push_all_with_policy(
            vec![note.clone()],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        let base = manager.generation();
        manager.poll(now);
        assert_ne!(manager.generation(), base);
        // Consecutive duplicate merges into the showing row: visible.
        let base = manager.generation();
        manager.push_all_with_policy(
            vec![note],
            &policy(),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        assert_ne!(manager.generation(), base);
        assert!(manager.rows()[0].text.ends_with(" ×2"));
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
}

#[cfg(test)]
mod osd_cov_tests {
    use crate::session::osd::*;

    #[test]
    fn status_parts_split_label_and_value() {
        let (label, value) = OsdManager::status_parts(60.0, true, AppLanguage::English);
        assert_eq!(label, "Paused");
        assert_eq!(value, "60.0 fps");
        let (label, _) = OsdManager::status_parts(0.0, false, AppLanguage::Japanese);
        assert!(label.is_empty());
    }

    #[test]
    fn empty_manager_reports_idle() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        assert!(!manager.has_visible());
        assert!(manager.wakeup_deadline(now).is_none());
        assert!(!manager.pump_modal());
        assert_eq!(manager.overflow_dropped(), 0);
        assert!(manager.visible_row().is_none());
        manager.requeue_front("head".to_string());
        assert_eq!(manager.take_modal_batch().as_deref(), Some("head"));
        manager.modal_closed();
    }

    #[test]
    fn pump_target_delegates_to_manager() {
        use super::PumpTarget as _;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_notifications(
            vec![],
            &TransientPolicy::from(
                &nerust_settings_core::model::local::NotificationSettings::default(),
            ),
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        manager.poll_queue(now);
        assert!(!manager.pump_modal_queue());
    }
}
