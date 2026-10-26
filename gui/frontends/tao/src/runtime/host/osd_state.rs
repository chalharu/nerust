//! Native OSD service: layer sync, notification queue, and dialog flight.
//! Owned by `HostState`; the drain router uses only the narrow ports below.

use nerust_frontend_host::osd::{OsdLayerSync, OsdNotifications};

/// Native presentation state; history and pause ownership live in ApplicationHost.
pub(super) struct OsdService {
    pub(super) layer: OsdLayerSync,
    pub(super) modal: OsdNotifications,
}

impl OsdService {
    pub(super) fn new() -> Self {
        Self {
            layer: OsdLayerSync::new(),
            modal: OsdNotifications::new(),
        }
    }

    /// Drop the native dialog receiver; common close retires pause ownership.
    pub(super) fn close_all(&mut self) {
        self.modal.close();
    }

    /// Queued error batches awaiting a dialog (test read).
    #[cfg(test)]
    pub(super) fn modal_pending(&self) -> usize {
        self.modal.pending()
    }

    /// Whether an error dialog is currently showing (test read).
    #[cfg(test)]
    pub(super) fn modal_showing(&self) -> bool {
        self.modal.modal_pending()
    }

    /// Port for the modal drain: queued batch count.
    #[cfg(test)]
    pub(super) fn modal_queue_len(&self) -> usize {
        self.modal.pending()
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
        assert!(host.osd.modal.take_modal().is_none());
        assert!(host.core.poll_modal_ack(&mut host.osd.modal).is_none());
        assert!(!host.osd.modal_showing());
        assert!(host.history_key().is_none());
        host.osd.close_all();
        assert_eq!(host.osd.modal_pending(), 0);
    }
}
