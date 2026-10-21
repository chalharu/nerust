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

/// Queue caps for the OSD subsystem, in one home instead of scattered literals.
pub(crate) struct OsdLimits;

impl OsdLimits {
    /// Sequential display (one showing row) plus pending rows.
    pub(crate) const MAX_TRANSIENT_QUEUED: usize = 2;
    /// Sequential modal batches; overflow drops the oldest.
    pub(crate) const MAX_MODAL_QUEUED: usize = 8;
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

/// Modal half of [`OsdManager`](crate::session::osd::OsdManager): aggregation plus display queue.
#[derive(Debug, Default)]
pub(crate) struct ModalCenter {
    modal_box: ModalBox,
    /// Overflow warn episode flag. Re-arms when the modal queue
    /// fully drains (see the modal source below).
    warned_full: bool,
}

impl ModalCenter {
    pub(crate) fn set_presentation(&mut self, policy: TransientPolicy, language: AppLanguage) {
        if self.modal_box.presentation == Some((policy, language)) {
            return;
        }
        self.modal_box.presentation = Some((policy, language));
        let enabled = |line: &ModalLine| match line {
            ModalLine::Text(_) => true,
            ModalLine::Notification { level, .. } => policy.enabled(*level),
        };
        self.modal_box.batch.retain(enabled);
        for lines in &mut self.modal_box.queue {
            lines.retain(enabled);
        }
        self.modal_box.queue.retain(|lines| !lines.is_empty());
    }

    pub(crate) fn push_message(&mut self, message: Message, count: u32, level: NotificationLevel) {
        self.modal_box.batch.push(ModalLine::Notification {
            message,
            count,
            level,
        });
    }

    /// Preserve structured lines all the way to dialog delivery.
    pub(crate) fn pump_pending(&mut self) -> ModalPump {
        let lines = std::mem::take(&mut self.modal_box.batch);
        if lines.is_empty() {
            return ModalPump::Idle;
        }
        self.modal_box.queue.push_back(lines);
        ModalPump::Queued {
            dropped: self.modal_box.enforce_cap(),
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
        self.modal_box.push(text);
    }

    /// Take the next batch only when no dialog is showing. Re-arms
    /// the overflow warn episode when the queue fully drains.
    pub(crate) fn take_batch(&mut self) -> Option<String> {
        let next = self.modal_box.take_modal_batch();
        if next.is_some() && self.modal_box.modal_pending() == 0 {
            self.warned_full = false;
        }
        next
    }

    /// Ack the showing dialog. Re-arms the overflow warn episode when
    /// the queue fully drains.
    pub(crate) fn closed(&mut self) {
        self.modal_box.modal_closed();
        if self.modal_box.modal_pending() == 0 {
            self.warned_full = false;
        }
    }

    pub(crate) fn pending(&self) -> usize {
        self.modal_box.modal_pending()
    }

    /// Direct queue write for the sink side. Returns true on overflow.
    pub(crate) fn queue_direct(&mut self, detail: String) -> bool {
        self.modal_box.queue_modal(detail)
    }

    /// Direct requeue write for the sink side. Returns true on overflow.
    pub(crate) fn requeue_direct(&mut self, detail: String) -> bool {
        self.modal_box.requeue_front(detail)
    }
}

/// Modal error batches with sequential display pumping.
#[derive(Debug, Default)]
pub(crate) struct ModalBox {
    batch: Vec<ModalLine>,
    queue: std::collections::VecDeque<Vec<ModalLine>>,
    presentation: Option<(TransientPolicy, AppLanguage)>,
    open: bool,
    /// Total overflow losses. Pure fact, no I/O: the pump adapter
    /// decides logging from the delta (see `pump_modal`).
    pub(crate) dropped_total: u64,
}

/// Write side of the modal queue. Only the shell pump and the
/// owning frontend adapter implement this; renderers never see it.
pub trait ModalSink {
    fn queue_modal(&mut self, detail: String);
    fn requeue_front(&mut self, detail: String);
}

/// Read side of the modal queue. Dialog hosts take batches and
/// ack them; they cannot inject or reorder.
pub trait ModalSource {
    fn take_modal_batch(&mut self) -> Option<String>;
    fn modal_closed(&mut self);
    fn modal_pending(&self) -> usize;
}

impl ModalBox {
    /// Cap shared by queue and requeue paths (drop-oldest discipline).
    const MAX_QUEUED: usize = OsdLimits::MAX_MODAL_QUEUED;

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
    pub(crate) fn queue_modal(&mut self, detail: String) -> bool {
        self.queue.push_back(vec![ModalLine::Text(detail)]);
        self.enforce_cap()
    }

    /// Re-queue an interrupted head batch preserving display order.
    pub(crate) fn requeue_front(&mut self, detail: String) -> bool {
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
    pub(crate) fn take_modal_batch(&mut self) -> Option<String> {
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
                return Some(rendered.join("\n"));
            }
        }
        None
    }

    pub(crate) fn modal_closed(&mut self) {
        self.open = false;
    }

    pub(crate) fn modal_pending(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_modal_text_drains_raw_lines() {
        let mut modal = ModalBox::default();
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
        let mut modal = ModalBox::default();
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
        let mut center = ModalCenter::default();
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
            assert!(!center.queue_direct(format!("batch {i}")));
        }
        assert!(center.queue_direct("batch 8".to_string()));
    }

    #[test]
    fn modal_requeue_overflow_counts_losses() {
        let mut modal = ModalBox::default();
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
}

#[cfg(test)]
mod modal_cov_tests {
    use crate::session::osd_modal::*;

    #[test]
    fn overflow_flag_arms_and_rearms() {
        let mut center = ModalCenter::default();
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
        let mut center = ModalCenter::default();
        for i in 0..8 {
            assert!(!center.queue_direct(format!("b{i}")));
        }
        assert!(center.queue_direct("overflow".to_string()));
        assert!(center.requeue_direct("head".to_string()));
        assert_eq!(center.pending(), 8);
    }

    #[test]
    fn limits_match_documented_caps() {
        assert_eq!(OsdLimits::MAX_TRANSIENT_QUEUED, 2);
        assert_eq!(OsdLimits::MAX_MODAL_QUEUED, 8);
    }
}
