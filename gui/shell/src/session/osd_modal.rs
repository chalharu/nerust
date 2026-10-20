//! Modal error batches: sequential display pumping.
//! The shell pump and the owning frontend adapter are the only writers.

/// Queue caps for the OSD subsystem. One home so burst-cap tuning
/// stays grep-able instead of scattering literals across the queue
/// and modal boxes.
pub(crate) struct OsdLimits;

impl OsdLimits {
    /// Sequential display (one showing row) plus pending rows.
    pub(crate) const MAX_TRANSIENT_QUEUED: usize = 2;
    /// Sequential modal batches; overflow drops the oldest.
    pub(crate) const MAX_MODAL_QUEUED: usize = 8;
}

/// Pure outcome of one modal pump step. Data only: the caller
/// formats the aggregated batch and logs overflow, so this box never
/// touches `format!` or the log.
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

/// Modal half of [`OsdManager`](crate::session::osd::OsdManager):
/// aggregation plus the sequential display queue. Pure queue logic;
/// bullet formatting and overflow logging live at the render boundary
/// (the manager's `pump_modal`), never here.
#[derive(Debug, Default)]
pub(crate) struct ModalCenter {
    modal_box: ModalBox,
    /// Overflow warn episode flag. Re-arms when the modal queue
    /// fully drains (see the modal source below).
    warned_full: bool,
}

impl ModalCenter {
    /// Move the aggregated batch into the sequential queue, capped.
    /// Pure: returns the outcome for the caller to format and log.
    /// `detailed` is the pre-formatted batch text supplied by the
    /// caller; the box only stores it.
    pub(crate) fn pump(&mut self, detailed: Option<String>) -> ModalPump {
        match detailed {
            None => ModalPump::Idle,
            Some(detail) => {
                let dropped = self.modal_box.queue_modal(detail);
                ModalPump::Queued { dropped }
            }
        }
    }

    /// Record that one overflow episode was reported. Called by the
    /// render boundary after it logs, so repeats stay quiet until the
    /// queue fully drains and re-arms.
    pub(crate) fn note_overflow_reported(&mut self) {
        self.warned_full = true;
    }

    pub(crate) fn overflow_already_reported(&self) -> bool {
        self.warned_full
    }

    /// Take the pending raw lines as one display batch. The caller
    /// formats them (bullets) at the render boundary.
    pub(crate) fn take_batch_text(&mut self) -> Vec<String> {
        self.modal_box.take_modal_text()
    }

    /// Push one pre-routed raw line into the aggregation.
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

    /// Total modal overflow losses (test hook for the warn-once path).
    #[cfg(test)]
    pub(crate) fn dropped_total(&self) -> u64 {
        self.modal_box.dropped_total
    }
}

/// Modal error batches with sequential display pumping.
#[derive(Debug, Default)]
pub(crate) struct ModalBox {
    batch: Vec<String>,
    queue: std::collections::VecDeque<String>,
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

    pub(crate) fn push(&mut self, text: String) {
        self.batch.push(text);
    }

    pub(crate) fn take_batch(&mut self) -> Vec<String> {
        std::mem::take(&mut self.batch)
    }

    /// Take pending error lines as one display batch. Returns the raw
    /// lines: bullet formatting happens at the render boundary
    /// (`pump_modal`), so queued content stays plain data.
    pub(crate) fn take_modal_text(&mut self) -> Vec<String> {
        self.take_batch()
    }

    /// Queue one aggregated error batch for sequential display.
    /// Capped: a stuck-open dialog plus a per-tick error flood must
    /// not grow memory without bound. Drops the oldest, like the OSD
    /// pending cap (history keeps every event). Returns true when a
    /// batch was dropped.
    pub(crate) fn queue_modal(&mut self, detail: String) -> bool {
        let dropped = self.queue.len() >= Self::MAX_QUEUED;
        if dropped {
            self.queue.pop_front();
            self.dropped_total += 1;
        }
        self.queue.push_back(detail);
        dropped
    }

    /// Re-queue an interrupted head batch preserving display order.
    /// Overflow drops the oldest queued batch behind the head.
    pub(crate) fn requeue_front(&mut self, detail: String) -> bool {
        self.queue.push_front(detail);
        let mut dropped = false;
        while self.queue.len() > Self::MAX_QUEUED {
            self.queue.remove(1);
            self.dropped_total += 1;
            dropped = true;
        }
        dropped
    }

    /// Take the next batch only when no dialog is showing: frontends
    /// present one dialog at a time and ack via the modal source, so
    /// errors never stack.
    pub(crate) fn take_modal_batch(&mut self) -> Option<String> {
        if self.open {
            return None;
        }
        let next = self.queue.pop_front()?;
        self.open = true;
        Some(next)
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
        let idle = center.pump(None);
        assert_eq!(idle, ModalPump::Idle);
        assert!(matches!(idle, ModalPump::Idle));
        // Queued batches report without formatting or logging.
        center.push_text("raw line".to_string());
        assert_eq!(
            center.pump(Some("raw line".to_string())),
            ModalPump::Queued { dropped: false }
        );
        assert!(matches!(
            ModalPump::Queued { dropped: false },
            ModalPump::Queued { .. }
        ));
        // Take/close drain through the center and re-arm the flag.
        assert_eq!(center.pending(), 1);
        assert_eq!(center.take_batch().as_deref(), Some("raw line"));
        assert_eq!(center.pending(), 0);
        center.closed();
        // Overflow records the drop for the caller to log.
        for i in 0..9 {
            center.queue_direct(format!("batch {i}"));
        }
        assert_eq!(center.dropped_total(), 1);
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
