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
