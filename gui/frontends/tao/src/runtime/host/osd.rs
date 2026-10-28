//! Native OSD effects; ApplicationHost owns notification and rendering state.

use std::time::Instant;

use tao::event::WindowEvent;
use wakui::aux::{AuxId, OpenRequest};
use wakui::osd::{ModalPresentation, ModalPresenter};

use super::HostState;
use super::window::WakeNeed;
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

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

impl HostState {
    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let Some(request) = self.reserve_aux_window(AuxKind::History) else {
            return;
        };
        let id = match request {
            OpenRequest::Create(id) => id,
            OpenRequest::Present(_) => {
                if let Some(id) = self.aux.route_id(AuxKind::History) {
                    self.sync_history_window(id);
                }
                return;
            }
            OpenRequest::Pending(_) => return,
        };
        if !self.prepare_widget(id) {
            return;
        }
        let (screen, mailbox) = match (self.core.widget_screen(id), self.core.widget_mailbox(id)) {
            (Some(screen), Some(mailbox)) => (screen, mailbox),
            _ => {
                self.core.cancel_widget(&mut self.aux.windows, id);
                return;
            }
        };
        let opened = self.create_aux_window(
            id,
            opener,
            crate::ports::AuxOpenParams::Controls {
                id,
                screen: Box::new(screen),
                mailbox,
            },
        );
        if opened {
            // Fresh window needs full content regardless of cache.
            self.sync_history_window(id);
        }
    }

    fn sync_history_window(&mut self, id: AuxId) {
        if self.core.history_changed() {
            self.core.widget_refresh(id);
        }
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        let id = self.aux.route_id(AuxKind::History);
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(id) = id {
                self.finish_widget_window(id, true);
            }
            return;
        }
        // Read-only viewer: shared aux-focus drive (arrival only
        // cancels a pending pause, never resumes).
        if let WindowEvent::Focused(focused) = event {
            self.on_focus_event(wakui::focus::FocusEvent::Aux(AuxKind::History, focused));
        }
        let domain = crate::aux::native::adapt_event(&event);
        let Some(outcome) = self.aux_on_event(AuxKind::History, domain) else {
            return;
        };
        if outcome.close {
            self.finish_widget_window(outcome.id.expect("routed generation"), true);
            return;
        }
        if outcome.submitted {
            let id = outcome.id.expect("routed generation");
            self.finish_widget_window(id, false);
            self.sync_history_window(id);
        }
    }

    /// A fresh destination has neither the layer nor the reserved strip.
    #[cfg(test)]
    pub(crate) fn poll_notifications(&mut self) {
        self.tick_osd_pump();
    }

    /// Execute only native effects after the application borrow ends.
    pub(super) fn tick_osd_pump(&mut self) -> WakeNeed {
        let turn = self.core.pump_osd(Instant::now());
        if let Some(presentation) = turn.modal {
            let token = presentation.token;
            let adapter = DialogPresenter {
                dialogs: self.dialogs.as_ref(),
                error_title: self.core.dialog_titles().error,
            };
            let presenter: &dyn ModalPresenter = &adapter;
            if let Some(receiver) = presenter.present_modal(presentation) {
                self.core.attach_osd_modal_receiver(token, receiver);
            }
        }
        if self.core.redraw_needed() {
            self.windows.request_redraw();
        }
        if self.aux.contains(AuxKind::History)
            && let Some(id) = self.aux.route_id(AuxKind::History)
        {
            self.sync_history_window(id);
        }
        // Receiver attachment adds a polling flight after the core's turn capture.
        WakeNeed {
            flight_pending: self.core.osd_flight_pending(),
            ..turn.wake
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;
    use wakui::focus::FocusEvent;

    #[test]
    fn parked_modal_has_ack_backstop_and_releases_only_its_owner() {
        use wakui::pause::PauseOwner;
        let mut host = TestHost::with_nes().host;
        host.load_test_rom();
        host.core.report_host_error("parked modal".into());
        host.core.note_main_departure();
        host.core.focus_event(FocusEvent::BurstEnd);
        let (ack, done) = std::sync::mpsc::channel();
        let presentation = host.core.pump_osd(Instant::now()).modal.unwrap();
        assert!(
            host.core
                .attach_osd_modal_receiver(presentation.token, done)
        );
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
        assert!(host.core.osd_wakeup(std::time::Instant::now()).is_none());
        let wake = host.tick_osd_pump();
        assert!(wake.osd_deadline.is_none());
        assert!(wake.live_tick.is_none());
        assert!(!wake.flight_pending);
    }

    #[test]
    fn close_retires_owned_modal_receiver_queue() {
        use wakui::osd::ModalAdvance;
        use wakui::pause::PauseOwner;
        let mut host = TestHost::new().host;
        host.core.set_pause_owner(PauseOwner::Settings, true);
        host.core.report_host_error("active on close".into());
        let presentation = host.core.pump_osd(Instant::now()).modal.unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(
            host.core
                .attach_osd_modal_receiver(presentation.token, receiver)
        );
        host.core.report_host_error("queued on close".into());
        host.core.pump_osd(Instant::now());
        assert!(host.core.osd_modal_pending());
        assert!(host.core.osd_flight_pending());
        host.prepare_close();
        assert!(sender.send(()).is_err());
        assert!(!host.core.osd_modal_pending());
        assert!(!host.core.osd_flight_pending());
        assert_eq!(host.core.osd_pending(), 0);
        assert!(!host.core.pause_held());
        assert_eq!(
            host.core.acknowledge_osd_modal(presentation.token),
            ModalAdvance::Ignored
        );
        host.prepare_close();
        assert!(host.core.pump_osd(Instant::now()).modal.is_none());
    }

    #[test]
    fn history_open_records_request_and_stays_closed() {
        use crate::aux::focus::AuxKind;
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_history(&opener);
        host.open_history(&opener);
        assert_eq!(opener.calls(), [OpenerCall::History, OpenerCall::History]);
        assert!(!host.aux.contains(AuxKind::History));
        assert!(host.core.history_changed());
    }

    #[test]
    fn pending_history_open_skips_creation_and_content_cache_updates() {
        use crate::test_opener::FakeOpener;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host.aux.request_open(AuxKind::History) else {
            panic!("expected reservation")
        };
        let opener = FakeOpener::failing("no display");
        host.open_history(&opener);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::History), Some(id));
        assert!(host.core.history_changed());
    }

    #[test]
    fn history_events_without_window_stay_total() {
        use tao::event::WindowEvent;
        let support = TestHost::new();
        let mut host = support.host;
        host.on_history_event(WindowEvent::Destroyed);
        host.on_history_event(WindowEvent::Focused(true));
        host.on_history_event(WindowEvent::Focused(false));
        assert!(host.core.history_changed());
        assert!(support.menu.updates().is_empty());
    }
}
