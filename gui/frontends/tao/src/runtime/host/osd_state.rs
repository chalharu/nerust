//! OSD service cluster: layer sync plus history plus modal queue.
//! Owned by `HostState`; the drain router uses only the narrow ports below.

use super::osd::ModalFlight;
use nerust_frontend_host::osd::{ModalAdvance, OsdLayerSync, OsdNotifications};
use nerust_frontend_kernel::Versioned;

/// History viewer sync: content key only. The window lives in the registry.
pub(super) struct HistoryViewer {
    pub(super) sync_key: Versioned<(u64, nerust_settings_core::model::language::AppLanguage)>,
}

/// Modal queue: error batches plus the in-flight dialog.
pub(super) struct ModalFlightQueue {
    pub(super) notifications: OsdNotifications,
    pub(super) flight: Option<ModalFlight>,
}

/// OSD service: layer sync plus history plus modal queue.
pub(super) struct OsdService {
    pub(super) layer: OsdLayerSync,
    pub(super) history: HistoryViewer,
    pub(super) modal: ModalFlightQueue,
}

impl OsdService {
    pub(super) fn new() -> Self {
        Self {
            layer: OsdLayerSync::new(),
            history: HistoryViewer {
                sync_key: Versioned::new(),
            },
            modal: ModalFlightQueue {
                notifications: OsdNotifications::new(),
                flight: None,
            },
        }
    }

    /// Close teardown: sync memos, dialog, and resume flag.
    pub(super) fn close_all(&mut self) {
        self.history.sync_key.clear();
        self.modal.flight.take();
    }

    /// Queued error batches awaiting a dialog (test read).
    #[cfg(test)]
    pub(super) fn modal_pending(&self) -> usize {
        self.modal.notifications.pending()
    }

    /// Test seam: simulate a dialog thread that died before acking.
    #[cfg(test)]
    pub(super) fn inject_modal_flight_for_test(&mut self, flight: ModalFlight) {
        self.modal.flight = Some(flight);
    }

    /// Whether an error dialog is currently showing (test read).
    #[cfg(test)]
    pub(super) fn modal_showing(&self) -> bool {
        self.modal.flight.is_some()
    }

    /// Last synced history content key (test read).
    #[cfg(test)]
    pub(super) fn history_key(
        &self,
    ) -> Option<(u64, nerust_settings_core::model::language::AppLanguage)> {
        self.history.sync_key.get().copied()
    }

    /// Port for the modal drain: next queued error batch, if any and no dialog is showing.
    pub(super) fn take_modal_batch(&mut self) -> Option<String> {
        if self.modal.flight.is_some() {
            return None;
        }
        self.modal.notifications.take_modal_batch()
    }

    /// Port for the modal drain: close the acked batch.
    pub(super) fn modal_closed(&mut self) -> ModalAdvance {
        self.modal.notifications.acknowledge_modal()
    }

    /// Port for the modal drain: queued batch count.
    #[cfg(test)]
    pub(super) fn modal_queue_len(&self) -> usize {
        self.modal.notifications.pending()
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    #[test]
    fn osd_service_ports_stay_total_while_idle() {
        let support = TestHost::new();
        let mut host = support.host;
        assert_eq!(host.osd.modal_pending(), 0);
        assert_eq!(host.osd.modal_queue_len(), 0);
        assert!(host.osd.take_modal_batch().is_none());
        host.osd.modal_closed();
        assert!(!host.osd.modal_showing());
        assert!(host.osd.history_key().is_none());
        host.osd.close_all();
        assert_eq!(host.osd.modal_pending(), 0);
    }
}
