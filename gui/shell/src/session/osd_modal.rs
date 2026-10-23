//! Modal error batches: sequential display pumping.
//! The shell pump and the owning frontend adapter are the only writers.

use super::{
    notification::{Message, NotificationLevel, NotificationRenderer, append_count},
    osd_transient::TransientPolicy,
};
use nerust_settings_core::model::language::AppLanguage;

#[derive(Debug)]
enum ModalLine {
    Text(String),
    Notification {
        message: Message,
        count: u32,
        level: NotificationLevel,
    },
}

impl ModalLine {
    fn render(&self, presentation: Option<(TransientPolicy, AppLanguage)>) -> Option<String> {
        match self {
            Self::Text(text) => Some(text.clone()),
            Self::Notification {
                message,
                count,
                level,
            } => {
                let (policy, language) = presentation?;
                policy.enabled(*level).then(|| {
                    append_count(
                        NotificationRenderer { language }.render_message(message),
                        *count,
                    )
                })
            }
        }
    }
}

/// Pure outcome of one modal pump step: data only, never touching `format!` or the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModalPump {
    /// Nothing queued.
    Idle,
    /// One batch moved to the sequential queue.
    Queued {
        /// Batches dropped to overflow while queueing (caller logs).
        dropped: bool,
    },
}

/// Modal error batches with sequential display pumping. Single queue gate:
/// aggregation plus the display queue live here, so cap and overflow policy
/// change in one place. Frontends take batches through [`OsdManager`].
#[derive(Debug, Default)]
pub struct ModalQueue {
    batch: Vec<ModalLine>,
    queue: std::collections::VecDeque<Vec<ModalLine>>,
    presentation: Option<(TransientPolicy, AppLanguage)>,
    open: bool,
    /// Batch handed out but not acked: interrupts hand it back structured.
    showing: Option<Vec<ModalLine>>,
    /// Total overflow losses. Pure fact, no I/O: the pump adapter
    /// decides logging from the delta (see `pump_modal`).
    pub(crate) dropped_total: u64,
    /// Overflow warn episode flag. Re-arms when the modal queue fully drains.
    warned_full: bool,
}

impl ModalQueue {
    /// Cap shared by queue and requeue paths (drop-oldest discipline).
    const MAX_QUEUED: usize = 8;

    pub(crate) fn set_presentation(&mut self, policy: TransientPolicy, language: AppLanguage) {
        if self.presentation == Some((policy, language)) {
            return;
        }
        self.presentation = Some((policy, language));
        let enabled = |line: &ModalLine| match line {
            ModalLine::Text(_) => true,
            ModalLine::Notification { level, .. } => policy.enabled(*level),
        };
        self.batch.retain(enabled);
        for lines in &mut self.queue {
            lines.retain(enabled);
        }
        self.queue.retain(|lines| !lines.is_empty());
        // The taken-but-unacked batch redelivers through the same filter:
        // an interrupt after a settings change must not resurrect lines the
        // new policy disables (NER-318). The open gate stays until ack.
        if let Some(lines) = self.showing.as_mut() {
            lines.retain(enabled);
            if lines.is_empty() {
                self.showing = None;
            }
        }
    }

    pub(crate) fn push_message(&mut self, message: Message, count: u32, level: NotificationLevel) {
        // Same-tick storm: fold into the tail instead of queueing a twin batch.
        if let Some(ModalLine::Notification {
            message: back,
            count: back_count,
            level: back_level,
        }) = self.batch.last_mut()
            && *back_level == level
            && back.coalescing_key() == message.coalescing_key()
        {
            *back_count = back_count.saturating_add(count);
            return;
        }
        self.batch.push(ModalLine::Notification {
            message,
            count,
            level,
        });
    }

    /// Preserve structured lines all the way to dialog delivery.
    pub(crate) fn pump_pending(&mut self) -> ModalPump {
        let lines = std::mem::take(&mut self.batch);
        if lines.is_empty() {
            return ModalPump::Idle;
        }
        // Cross-tick storm: a batch identical to the queue tail folds into it
        // instead of stacking another waiting dialog.
        if let Some(back) = self.queue.back_mut()
            && same_batch(back, &lines)
        {
            for (dst, src) in back.iter_mut().zip(&lines) {
                if let (
                    ModalLine::Notification {
                        count: dst_count, ..
                    },
                    ModalLine::Notification {
                        count: src_count, ..
                    },
                ) = (dst, src)
                {
                    *dst_count = dst_count.saturating_add(*src_count);
                }
            }
            return ModalPump::Queued { dropped: false };
        }
        self.queue.push_back(lines);
        ModalPump::Queued {
            dropped: self.enforce_cap(),
        }
    }

    /// Record that one overflow episode was reported; repeats stay quiet until the queue drains.
    pub(crate) fn note_overflow_reported(&mut self) {
        self.warned_full = true;
    }

    pub(crate) fn overflow_already_reported(&self) -> bool {
        self.warned_full
    }

    /// Push one pre-routed raw line into the aggregation.
    #[cfg(test)]
    pub(crate) fn push_text(&mut self, text: String) {
        self.push(text);
    }

    /// Take the next batch only when no dialog is showing. Re-arms
    /// the overflow warn episode when the queue fully drains.
    pub(crate) fn take_batch(&mut self) -> Option<String> {
        let next = self.take_modal_batch();
        if next.is_some() && self.modal_pending() == 0 {
            self.warned_full = false;
        }
        next
    }

    /// Ack the showing dialog. Re-arms the overflow warn episode when
    /// the queue fully drains.
    pub(crate) fn closed(&mut self) {
        self.modal_closed();
        if self.modal_pending() == 0 {
            self.warned_full = false;
        }
    }

    pub(crate) fn pending(&self) -> usize {
        self.modal_pending()
    }
}

impl ModalQueue {
    #[cfg(test)]
    pub(crate) fn push(&mut self, text: String) {
        self.batch.push(ModalLine::Text(text));
    }

    /// Take pending error lines as one display batch, as raw lines for the boundary to format.
    #[cfg(test)]
    pub(crate) fn take_modal_text(&mut self) -> Vec<String> {
        std::mem::take(&mut self.batch)
            .iter()
            .filter_map(|line| line.render(self.presentation))
            .collect()
    }

    /// Single cap enforcement: drop-oldest, count losses.
    fn enforce_cap(&mut self) -> bool {
        let mut dropped = false;
        while self.queue.len() > Self::MAX_QUEUED {
            self.queue.pop_front();
            self.dropped_total += 1;
            dropped = true;
        }
        dropped
    }

    /// Queue one aggregated error batch for sequential display, capped.
    /// Consecutive identical details coalesce: an error storm queues once.
    pub fn queue_modal(&mut self, detail: String) -> bool {
        if self.queue.back().is_some_and(
            |lines| matches!(lines.as_slice(), [ModalLine::Text(back)] if back == &detail),
        ) {
            return false;
        }
        self.queue.push_back(vec![ModalLine::Text(detail)]);
        self.enforce_cap()
    }

    /// Re-queue an interrupted head batch preserving display order.
    pub fn requeue_front(&mut self, detail: String) -> bool {
        self.queue.push_front(vec![ModalLine::Text(detail)]);
        let mut dropped = false;
        while self.queue.len() > Self::MAX_QUEUED {
            self.queue.remove(1);
            self.dropped_total += 1;
            dropped = true;
        }
        dropped
    }

    /// Take the next batch only when no dialog is showing, so errors never stack.
    pub fn take_modal_batch(&mut self) -> Option<String> {
        if self.open {
            return None;
        }
        while let Some(lines) = self.queue.pop_front() {
            let structured = lines
                .iter()
                .any(|line| matches!(line, ModalLine::Notification { .. }));
            let rendered: Vec<_> = lines
                .iter()
                .filter_map(|line| line.render(self.presentation))
                .map(|text| {
                    if structured {
                        format!("• {text}")
                    } else {
                        text
                    }
                })
                .collect();
            if !rendered.is_empty() {
                self.open = true;
                // Keep structured ownership until ack: an interrupt hands
                // the lines back instead of a pre-rendered string (NER-318).
                self.showing = Some(lines);
                return Some(rendered.join("\n"));
            }
        }
        None
    }

    pub fn modal_closed(&mut self) {
        self.open = false;
        self.showing = None;
    }

    /// Hand the unacked showing batch back to the front, preserving its
    /// structure so redelivery re-renders under current settings. No-op
    /// when nothing was taken. Same drop-oldest cap as the other requeue
    /// leg: an interrupt storm must not grow the queue without bound.
    pub fn requeue_showing(&mut self) -> bool {
        let Some(lines) = self.showing.take() else {
            return false;
        };
        self.open = false;
        self.queue.push_front(lines);
        while self.queue.len() > Self::MAX_QUEUED {
            self.queue.remove(1);
            self.dropped_total += 1;
        }
        true
    }

    pub fn modal_pending(&self) -> usize {
        self.queue.len()
    }
}

/// Same shape and identity line by line: level plus message key match, so a
/// repeated failure folds into one waiting dialog without losing its count.
fn same_batch(a: &[ModalLine], b: &[ModalLine]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (
                ModalLine::Notification {
                    message: a_message,
                    level: a_level,
                    ..
                },
                ModalLine::Notification {
                    message: b_message,
                    level: b_level,
                    ..
                },
            ) => a_level == b_level && a_message.coalescing_key() == b_message.coalescing_key(),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::notification::Message;
    use nerust_settings_core::model::language::AppLanguage;

    fn test_policy(warning_enabled: bool) -> TransientPolicy {
        TransientPolicy {
            info_enabled: true,
            warning_enabled,
            error_enabled: true,
            info_duration_ms: 2_000,
            warning_duration_ms: 2_000,
            info_rgb: (0, 0, 0),
            warning_rgb: (0, 0, 0),
            error_rgb: (200, 0, 0),
            error_modal: false,
        }
    }

    /// NER-318: an interrupted batch hands structured lines back, so the
    /// next take re-renders under current settings instead of replaying
    /// the pre-rendered text.
    #[test]
    fn interrupted_batch_rerenders_under_new_settings() {
        let mut modal = ModalQueue::default();
        modal.set_presentation(test_policy(true), AppLanguage::English);
        modal.push_message(
            Message::persistence_unavailable(),
            1,
            NotificationLevel::Warning,
        );
        assert!(matches!(modal.pump_pending(), ModalPump::Queued { .. }));
        let first = modal.take_modal_batch().expect("batch");
        assert!(first.contains("Saves unavailable"), "{first}");
        // Interrupt before ack: the structured batch goes back, not text.
        assert!(modal.requeue_showing());
        // New settings: Japanese, warnings still on. Redelivery translates.
        modal.set_presentation(test_policy(true), AppLanguage::Japanese);
        let second = modal.take_modal_batch().expect("redelivery");
        assert!(second.contains("保存機能が利用できません"), "{second}");
        // New settings: Japanese, warnings off. Redelivery is suppressed.
        modal.requeue_showing();
        modal.set_presentation(test_policy(false), AppLanguage::Japanese);
        assert_eq!(modal.take_modal_batch(), None);
    }

    /// NER-318: a settings change while a batch is taken refilters the
    /// showing batch too. An interrupt afterwards redelivers nothing the
    /// new policy disables, instead of the stale pre-change lines.
    #[test]
    fn set_presentation_refilters_taken_showing_batch() {
        let mut modal = ModalQueue::default();
        modal.set_presentation(test_policy(true), AppLanguage::English);
        modal.push_message(
            Message::persistence_unavailable(),
            1,
            NotificationLevel::Warning,
        );
        assert!(matches!(modal.pump_pending(), ModalPump::Queued { .. }));
        let first = modal.take_modal_batch().expect("batch");
        assert!(first.contains("Saves unavailable"), "{first}");
        // Settings change lands while the dialog is still up: Japanese,
        // warnings off. The taken batch loses its only line, so a later
        // interrupt has nothing to hand back and redelivery stays silent.
        modal.set_presentation(test_policy(false), AppLanguage::Japanese);
        assert!(!modal.requeue_showing());
        assert_eq!(modal.take_modal_batch(), None);
    }

    /// NER-318: requeueing the showing batch past a full queue drops the
    /// oldest waiting batch, like the other requeue leg.
    #[test]
    fn requeue_showing_enforces_queue_cap() {
        let mut modal = ModalQueue::default();
        for i in 0..8 {
            modal.queue_modal(format!("batch {i}"));
        }
        modal.take_modal_batch().expect("head batch");
        modal.queue_modal("refill".to_string());
        assert_eq!(modal.modal_pending(), 8);
        assert!(modal.requeue_showing());
        assert_eq!(modal.modal_pending(), 8);
        assert_eq!(modal.dropped_total, 1);
    }

    #[test]
    fn take_modal_text_drains_raw_lines() {
        let mut modal = ModalQueue::default();
        assert!(modal.take_modal_text().is_empty());
        modal.push("first".to_string());
        modal.push("second".to_string());
        // Raw lines: bullet formatting lives at the render boundary
        // (`pump_modal`), not in the box.
        assert_eq!(modal.take_modal_text(), vec!["first", "second"]);
        assert!(modal.take_modal_text().is_empty());
    }

    #[test]
    fn modal_overflow_counts_losses() {
        let mut modal = ModalQueue::default();
        for i in 0..9 {
            modal.queue_modal(format!("batch {i}"));
        }
        // Nine batches into a cap of eight: one loss recorded.
        assert_eq!(modal.dropped_total, 1);
        assert_eq!(modal.modal_pending(), 8);
        // Drain every batch through take and close.
        while modal.modal_pending() > 0 {
            modal.take_modal_batch().expect("queued batch");
            modal.modal_closed();
        }
        // A fresh overflow episode counts again.
        for i in 0..9 {
            modal.queue_modal(format!("again {i}"));
        }
        assert_eq!(modal.dropped_total, 2);
    }

    #[test]
    fn modal_center_pump_is_pure_data() {
        let mut center = ModalQueue::default();
        // Idle without a batch.
        let idle = center.pump_pending();
        assert_eq!(idle, ModalPump::Idle);
        // Queued batches report without formatting or logging.
        center.push_text("raw line".to_string());
        assert_eq!(center.pump_pending(), ModalPump::Queued { dropped: false });
        // Take/close drain through the center and re-arm the flag.
        assert_eq!(center.pending(), 1);
        assert_eq!(center.take_batch().as_deref(), Some("raw line"));
        assert_eq!(center.pending(), 0);
        center.closed();
        // Overflow records the drop for the caller to log: the ninth batch overflows.
        for i in 0..8 {
            assert!(!center.queue_modal(format!("batch {i}")));
        }
        assert!(center.queue_modal("batch 8".to_string()));
    }

    #[test]
    fn modal_requeue_overflow_counts_losses() {
        let mut modal = ModalQueue::default();
        for i in 0..8 {
            modal.queue_modal(format!("batch {i}"));
        }
        let taken = modal.take_modal_batch().expect("head batch");
        // Refill while the head is out, then suspend-requeue it back:
        // both overflow legs record a loss.
        for i in 0..2 {
            modal.queue_modal(format!("refill {i}"));
        }
        modal.requeue_front(taken);
        assert_eq!(modal.dropped_total, 2);
        assert_eq!(modal.modal_pending(), 8);
    }

    #[test]
    fn repeated_structured_failures_fold_into_one_waiting_dialog() {
        use crate::session::notification::{Message, MessageKey, NotificationLevel};
        let mut center = ModalQueue::default();
        // Same tick: one tail entry, counts add.
        center.push_message(
            Message::unit(MessageKey::SlotFailed),
            1,
            NotificationLevel::Error,
        );
        center.push_message(
            Message::unit(MessageKey::SlotFailed),
            2,
            NotificationLevel::Error,
        );
        assert_eq!(center.pump_pending(), ModalPump::Queued { dropped: false });
        assert_eq!(center.pending(), 1);
        // Next tick: an identical batch folds into the waiting tail.
        center.push_message(
            Message::unit(MessageKey::SlotFailed),
            4,
            NotificationLevel::Error,
        );
        assert_eq!(center.pump_pending(), ModalPump::Queued { dropped: false });
        assert_eq!(center.pending(), 1);
        // Different identity still queues separately.
        center.push_message(
            Message::unit(MessageKey::Resumed),
            1,
            NotificationLevel::Error,
        );
        assert_eq!(center.pump_pending(), ModalPump::Queued { dropped: false });
        assert_eq!(center.pending(), 2);
    }
}

#[cfg(test)]
mod modal_cov_tests {
    use crate::session::osd_modal::*;

    #[test]
    fn overflow_flag_arms_and_rearms() {
        let mut center = ModalQueue::default();
        assert!(!center.overflow_already_reported());
        center.note_overflow_reported();
        assert!(center.overflow_already_reported());
        center.push_text("x".to_string());
        assert_eq!(center.pump_pending(), ModalPump::Queued { dropped: false });
        assert_eq!(center.take_batch().as_deref(), Some("x"));
        center.closed();
        assert!(!center.overflow_already_reported());
    }

    #[test]
    fn direct_queue_reports_overflow() {
        let mut center = ModalQueue::default();
        for i in 0..8 {
            assert!(!center.queue_modal(format!("b{i}")));
        }
        assert!(center.queue_modal("overflow".to_string()));
        assert!(center.requeue_front("head".to_string()));
        assert_eq!(center.pending(), 8);
    }

    #[test]
    fn modal_limit_matches_documented_cap() {
        assert_eq!(ModalQueue::MAX_QUEUED, 8);
    }
}
