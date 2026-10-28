use std::{cell::RefCell, sync::mpsc, time::Duration};

use nerust_gui_shell::session::{access::PauseControl, commands::SessionCommand};
use nerust_render_traits::{ReservedStrip, renderer::OsdLayer};

use super::*;
use crate::{
    application::tests::Fixture,
    osd::{ModalPresentation, ModalPresenter, OsdLayerUpdate, OsdVisitor},
};

fn size() -> SurfaceSize {
    SurfaceSize::new(320, 240)
}

fn error(host: &mut ApplicationHost, detail: &str, now: Instant) -> OsdTurn {
    host.report_host_error(detail.into());
    let ctx = host.frame_ctx();
    host.pump_osd(&ctx, now)
}

fn status(host: &mut ApplicationHost, enabled: bool) -> FrameCtx {
    let mut settings = host.settings_snapshot().clone();
    settings.local.notifications.status_enabled = enabled;
    host.apply_settings(settings).unwrap();
    host.frame_ctx()
}

#[test]
fn coreless_idle_owned_pump_sleeps_and_preparation_is_empty_after_initial_strip() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let now = Instant::now();
    let ctx = host.frame_ctx();
    host.frame_presented(0);
    let turn = host.pump_osd(&ctx, now);
    assert!(!turn.changed && turn.modal.is_none());
    assert_eq!(turn.wake, WakeNeed::default());
    assert!(!host.redraw_needed());
    assert!(!host.pause_held());
    assert!(!host.osd_modal_pending() && !host.osd_flight_pending());
    assert_eq!(host.osd_pending(), 0);
    assert_eq!(host.osd_wakeup(now), None);
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, now),
        OsdUpdate {
            strip: Some(ReservedStrip::default()),
            ..Default::default()
        }
    );
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, now),
        OsdUpdate::default()
    );
    assert!(!host.osd_layer_live());
}

struct ReentrantOutput<'a> {
    fixture: &'a RefCell<Fixture>,
    calls: Vec<&'static str>,
}

impl OsdVisitor for ReentrantOutput<'_> {
    fn reserved_strip(&mut self, _: ReservedStrip) {
        self.fixture.borrow_mut().core.note_redraw();
        self.calls.push("strip");
    }
    fn layer(&mut self, layer: Option<OsdLayer>) {
        assert_eq!(self.fixture.borrow().core.osd_layer_live(), layer.is_some());
        self.fixture.borrow_mut().core.note_redraw();
        self.calls
            .push(if layer.is_some() { "set" } else { "clear" });
    }
}

#[test]
fn dyn_visitor_reborrows_application_and_visits_strip_before_layer_skipping_empty() {
    let fixture = RefCell::new(Fixture::new());
    let now = Instant::now();
    let update = {
        let mut state = fixture.borrow_mut();
        let ctx = status(&mut state.core, true);
        state.core.prepare_osd(size(), 480, &ctx, now)
    };
    let mut output = ReentrantOutput {
        fixture: &fixture,
        calls: vec![],
    };
    let port: &mut dyn OsdVisitor = &mut output;
    update.visit(port);
    OsdUpdate::default().visit(port);
    assert_eq!(output.calls, ["strip", "set"]);
    let update = {
        let mut state = fixture.borrow_mut();
        let ctx = status(&mut state.core, false);
        state.core.prepare_osd(size(), 480, &ctx, now)
    };
    update.visit(&mut output);
    assert_eq!(output.calls, ["strip", "set", "clear"]);
}

#[test]
fn owned_output_replacement_resize_and_zero_extent_republish_only_when_needed() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let ctx = status(host, true);
    let now = Instant::now();
    let first = host.prepare_osd(size(), 480, &ctx, now);
    assert!(matches!(first.layer, OsdLayerUpdate::Set(_)));
    assert!(host.osd_layer_live());
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, now),
        OsdUpdate::default()
    );
    host.invalidate_osd();
    assert!(!host.osd_layer_live());
    assert_eq!(host.prepare_osd(size(), 480, &ctx, now), first);
    let resized = host.prepare_osd(SurfaceSize::new(640, 480), 960, &ctx, now);
    assert!(matches!(resized.layer, OsdLayerUpdate::Set(_)));
    let zero = host.prepare_osd(SurfaceSize::new(0, 0), 960, &ctx, now);
    assert_eq!(zero.layer, OsdLayerUpdate::Clear);
    assert!(!host.osd_layer_live());
    assert!(matches!(
        host.prepare_osd(size(), 480, &ctx, now).layer,
        OsdLayerUpdate::Set(_)
    ));
}

#[test]
fn owned_receiver_ack_disconnect_chain_and_stale_callbacks_preserve_other_owners() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let now = Instant::now();
    let owners = [PauseOwner::Focus, PauseOwner::Settings, PauseOwner::User];
    for owner in owners {
        host.set_pause_owner(owner, true);
    }
    let first = error(host, "first", now).modal.unwrap();
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(!host.osd_flight_pending());
    let (sender, receiver) = mpsc::channel();
    assert!(host.attach_osd_modal_receiver(first.token, receiver));
    let (_duplicate, receiver) = mpsc::channel();
    assert!(!host.attach_osd_modal_receiver(first.token, receiver));
    let waiting = error(host, "second", now);
    assert!(waiting.modal.is_none() && waiting.wake.flight_pending);
    assert_eq!(host.osd_pending(), 1);
    sender.send(()).unwrap();
    sender.send(()).unwrap();
    let ctx = host.frame_ctx();
    let chained = host.pump_osd(&ctx, now);
    let second = chained.modal.unwrap();
    assert!(second.detail.contains("second"));
    assert_ne!(first.token, second.token);
    assert!(!chained.wake.flight_pending);
    assert!(host.owns_pause(PauseOwner::Modal));
    assert_eq!(
        host.acknowledge_osd_modal(first.token),
        ModalAdvance::Ignored
    );
    let (_late, receiver) = mpsc::channel();
    assert!(!host.attach_osd_modal_receiver(first.token, receiver));
    let (dead, receiver) = mpsc::channel();
    assert!(host.attach_osd_modal_receiver(second.token, receiver));
    drop(dead);
    // Notification is enqueued before polling the disconnect in this very turn.
    let third = error(host, "third", now).modal.unwrap();
    assert!(third.detail.contains("third"));
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(
        host.pump_osd(&ctx, now).modal.is_none(),
        "taken exactly once"
    );
    assert_eq!(
        host.acknowledge_osd_modal(third.token),
        ModalAdvance::Drained
    );
    assert_eq!(
        host.acknowledge_osd_modal(third.token),
        ModalAdvance::Ignored
    );
    assert!(!host.osd_modal_pending() && !host.osd_flight_pending());
    assert!(!host.owns_pause(PauseOwner::Modal));
    for owner in owners {
        assert!(host.owns_pause(owner));
    }
    assert!(host.pause_held());
}

#[test]
fn owned_final_receiver_ack_or_disconnect_releases_pause_and_flight_before_return() {
    for disconnect in [false, true] {
        let mut fixture = Fixture::new();
        fixture.load();
        let host = &mut fixture.core;
        let now = Instant::now();
        let effect = error(host, "final worker", now).modal.unwrap();
        let (sender, receiver) = mpsc::channel();
        assert!(host.attach_osd_modal_receiver(effect.token, receiver));
        assert!(host.tick().paused);
        if !disconnect {
            sender.send(()).unwrap();
        }
        drop(sender);
        host.frame_presented(0);
        let ctx = host.frame_ctx();
        let turn = host.pump_osd(&ctx, now);
        assert!(ctx.tick().paused, "capture predates ACK");
        assert!(!host.tick().paused, "late truth observes committed release");
        assert!(!host.pause_held() && !host.osd_modal_pending());
        assert!(!turn.wake.flight_pending && turn.modal.is_none());
        assert!(host.redraw_needed());
        assert_eq!(
            host.acknowledge_osd_modal(effect.token),
            ModalAdvance::Ignored
        );
    }
}

struct SynchronousModal<'a>(&'a RefCell<Fixture>);

impl ModalPresenter for SynchronousModal<'_> {
    fn present_modal(&self, effect: ModalPresentation) -> Option<mpsc::Receiver<()>> {
        let mut fixture = self.0.borrow_mut();
        assert!(fixture.core.owns_pause(PauseOwner::Modal));
        assert_eq!(
            fixture.core.acknowledge_osd_modal(effect.token),
            ModalAdvance::Drained
        );
        None
    }
}

#[test]
fn dyn_modal_port_synchronously_reborrows_host_before_receiver_attachment() {
    let fixture = RefCell::new(Fixture::new());
    let effect = error(&mut fixture.borrow_mut().core, "callback", Instant::now())
        .modal
        .unwrap();
    let token = effect.token;
    let native = SynchronousModal(&fixture);
    let port: &dyn ModalPresenter = &native;
    assert!(port.present_modal(effect).is_none());
    let mut fixture = fixture.borrow_mut();
    let (_sender, receiver) = mpsc::channel();
    assert!(!fixture.core.attach_osd_modal_receiver(token, receiver));
    assert!(!fixture.core.pause_held());
}

#[test]
fn close_retires_owned_receivers_queue_renderer_and_late_effects() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let ctx = status(host, true);
    let now = Instant::now();
    assert!(matches!(
        host.prepare_osd(size(), 480, &ctx, now).layer,
        OsdLayerUpdate::Set(_)
    ));
    let first = error(host, "active", now).modal.unwrap();
    let (sender, receiver) = mpsc::channel();
    assert!(host.attach_osd_modal_receiver(first.token, receiver));
    assert!(error(host, "queued", now).modal.is_none());
    assert_eq!(host.osd_pending(), 1);
    assert!(host.begin_close());
    assert!(sender.send(()).is_err());
    assert!(!host.osd_layer_live() && !host.osd_modal_pending() && !host.osd_flight_pending());
    assert_eq!(host.osd_pending(), 0);
    assert!(!host.pause_held());
    host.frame_presented(0);
    host.report_host_error("after close".into());
    let turn = host.pump_osd(&ctx, now);
    assert!(!turn.changed && turn.modal.is_none());
    assert_eq!(turn.wake, WakeNeed::default());
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, now),
        OsdUpdate::default()
    );
    assert_eq!(
        host.acknowledge_osd_modal(first.token),
        ModalAdvance::Ignored
    );
    let (_sender, receiver) = mpsc::channel();
    assert!(!host.attach_osd_modal_receiver(first.token, receiver));
    host.invalidate_osd();
    assert!(!host.redraw_needed() && !host.pause_held());
    assert_eq!(host.osd_wakeup(now), None);
    assert!(!host.begin_close());
}

#[test]
fn owned_transient_expiry_arms_wake_marks_redraw_and_clears_exactly_once() {
    let mut fixture = Fixture::new();
    fixture.load();
    let host = &mut fixture.core;
    host.pause();
    let ctx = host.frame_ctx();
    let now = Instant::now();
    let turn = host.pump_osd(&ctx, now);
    assert!(turn.changed && turn.modal.is_none());
    let deadline = turn.wake.osd_deadline.expect("visible pause notification");
    assert_eq!(host.osd_wakeup(now), Some(deadline));
    assert!(host.redraw_needed());
    assert!(matches!(
        host.prepare_osd(size(), 480, &ctx, now).layer,
        OsdLayerUpdate::Set(_)
    ));
    host.frame_presented(0);
    let steady = host.pump_osd(&ctx, now + Duration::from_millis(1));
    assert!(!steady.changed);
    assert!(
        host.redraw_needed(),
        "visible deadline requests redraw even without generation change"
    );
    host.frame_presented(0);
    let expired = host.pump_osd(&ctx, deadline);
    assert!(expired.changed && host.redraw_needed());
    assert_eq!(expired.wake.osd_deadline, None);
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, deadline).layer,
        OsdLayerUpdate::Clear
    );
    assert!(!host.osd_layer_live());
    assert_eq!(
        host.prepare_osd(size(), 480, &ctx, deadline),
        OsdUpdate::default()
    );
}

#[test]
fn owned_modal_adopts_independent_stop_but_idle_does_not_resume_it() {
    let mut fixture = Fixture::new();
    fixture.load();
    let host = &mut fixture.core;
    host.session_mut()
        .run_command(SessionCommand::Pause)
        .unwrap();
    let ctx = host.frame_ctx();
    let now = Instant::now();
    host.pump_osd(&ctx, now);
    assert!(host.tick().paused && !host.pause_held());
    let effect = error(host, "adopt independent pause", now).modal.unwrap();
    assert!(host.owns_pause(PauseOwner::User));
    assert_eq!(
        host.acknowledge_osd_modal(effect.token),
        ModalAdvance::Drained
    );
    assert!(host.tick().paused);
    assert!(!host.owns_pause(PauseOwner::Modal));
}
