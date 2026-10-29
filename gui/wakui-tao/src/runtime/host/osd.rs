//! Native dialog effects; the overlay layer lives renderer-side.

use std::time::Instant;

use wakui_core::dialog::{ModalPresentation, ModalPresenter};

use super::HostState;
use super::window::WakeNeed;

struct DialogPresenter<'a> {
    dialogs: &'a dyn crate::ports::DialogPorts,
    error_title: String,
}

impl ModalPresenter for DialogPresenter<'_> {
    fn present_modal(
        &self,
        presentation: ModalPresentation,
    ) -> Option<std::sync::mpsc::Receiver<()>> {
        Some(
            self.dialogs
                .show_error_async(self.error_title.clone(), presentation.detail),
        )
    }
}

impl<K: Copy + Eq + std::hash::Hash + 'static> HostState<K> {
    /// A fresh destination has neither the layer nor the reserved strip.
    #[cfg(test)]
    pub(crate) fn poll_notifications(&mut self) {
        self.tick_osd_pump();
    }

    /// Execute only native effects after the application borrow ends.
    pub(super) fn tick_osd_pump(&mut self) -> WakeNeed {
        let turn = self.core.pump_dialog(Instant::now());
        if let Some(presentation) = turn.modal {
            let token = presentation.token;
            let adapter = DialogPresenter {
                dialogs: self.dialogs.as_ref(),
                error_title: self.core.dialog_titles().error,
            };
            let presenter: &dyn ModalPresenter = &adapter;
            if let Some(receiver) = presenter.present_modal(presentation) {
                self.core.attach_dialog_receiver(token, receiver);
            }
        }
        if self.core.redraw_needed() {
            self.windows.request_redraw();
        }
        // Widget refresh runs in the aux pump (`pump_widgets`); dialog only
        // reports its own flights here.
        // Receiver attachment adds a polling flight after the core's turn capture.
        WakeNeed {
            flight_pending: self.core.dialog_flight_pending(),
            ..turn.wake
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;
    use wakui_core::focus::FocusEvent;

    #[test]
    fn parked_modal_has_ack_backstop_and_releases_only_its_owner() {
        use wakui_core::pause::PauseOwner;
        let mut host = TestHost::with_nes().host;
        host.load_test_rom();
        host.core.report_host_error("parked modal".into());
        host.core.note_main_departure();
        host.core.focus_event(FocusEvent::BurstEnd);
        let (ack, done) = std::sync::mpsc::channel();
        let presentation = host.core.pump_dialog(Instant::now()).modal.unwrap();
        assert!(host.core.attach_dialog_receiver(presentation.token, done));
        let wake = host.tick_osd_pump();
        assert!(
            wake.flight_pending,
            "ACK must wake a paused, unfocused loop"
        );
        ack.send(()).unwrap();
        host.tick_osd_pump();
        assert!(host.core.tick_paused(), "focus still holds");
        assert!(!host.core.owns_pause(PauseOwner::Modal));
        host.core.release_focus_pause();
        assert!(!host.core.tick_paused());
    }

    #[test]
    fn idle_pump_reports_no_wake() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.core.dialog_wakeup(std::time::Instant::now()).is_none());
        let wake = host.tick_osd_pump();
        assert!(wake.overlay_deadline.is_none());
        assert!(wake.live_tick.is_none());
        assert!(!wake.flight_pending);
    }

    #[test]
    fn close_retires_owned_modal_receiver_queue() {
        use wakui_core::dialog::ModalAdvance;
        use wakui_core::pause::PauseOwner;
        let mut host = TestHost::new().host;
        host.core.set_pause_owner(PauseOwner::Settings, true);
        host.core.report_host_error("active on close".into());
        let presentation = host.core.pump_dialog(Instant::now()).modal.unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(
            host.core
                .attach_dialog_receiver(presentation.token, receiver)
        );
        host.core.report_host_error("queued on close".into());
        host.core.pump_dialog(Instant::now());
        assert!(host.core.dialog_pending());
        assert!(host.core.dialog_flight_pending());
        host.prepare_close();
        assert!(sender.send(()).is_err());
        assert!(!host.core.dialog_pending());
        assert!(!host.core.dialog_flight_pending());
        assert_eq!(host.core.dialog_queue_len(), 0);
        assert!(!host.core.pause_held());
        assert_eq!(
            host.core.acknowledge_dialog(presentation.token),
            ModalAdvance::Ignored
        );
        host.prepare_close();
        assert!(host.core.pump_dialog(Instant::now()).modal.is_none());
    }

    #[test]
    fn history_open_records_request_and_stays_closed() {
        use crate::test_opener::{FakeOpener, OpenerCall};
        use crate::testkit::TestAux;
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_aux_window(TestAux::History, &opener);
        host.open_aux_window(TestAux::History, &opener);
        assert_eq!(opener.calls(), [OpenerCall::History, OpenerCall::History]);
        assert!(!host.aux.contains(TestAux::History));
        assert!(host.core.history_changed());
    }

    #[test]
    fn pending_history_open_skips_creation_and_content_cache_updates() {
        use crate::test_opener::FakeOpener;
        use crate::testkit::TestAux;
        use wakui_core::aux::OpenRequest;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host
            .aux
            .request_open(TestAux::History, TestAux::History.title())
        else {
            panic!("expected reservation")
        };
        let opener = FakeOpener::failing("no display");
        host.open_aux_window(TestAux::History, &opener);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(TestAux::History), Some(id));
        assert!(host.core.history_changed());
    }
}
