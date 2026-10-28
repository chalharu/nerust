//! Debugger and live-follow orchestration for `HostState`.
//! Drains, flights, aux-window management, and live PPU follow.

use nerust_frontend_host::aux::OpenRequest;
use nerust_frontend_viewdefs::application::DebugView;
use nerust_frontend_viewdefs::debugger::{AppliedDrain, AppliedLive};

use super::host::{HostState, WakeNeed};
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

struct NativeDebugView<'a> {
    aux: &'a mut crate::aux::registry::AuxRegistry,
    applied: bool,
}

impl DebugView for NativeDebugView<'_> {
    fn apply_drain(&mut self, output: AppliedDrain) {
        self.aux.debug_apply(output);
        self.applied = true;
    }

    fn apply_live(&mut self, live: AppliedLive) {
        if self.aux.debug_apply_live(live) {
            self.aux.ppu_sync();
        }
    }
}

impl HostState {
    /// Live follow is active: both aux windows open on a running loaded core.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.core
            .live_follow_active(self.is_debug_open(), self.is_ppu_open())
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Debug pump for the control-flow pass: live follow plus drain polling.
    pub(super) fn tick_debug_pump(&mut self) -> WakeNeed {
        let debug_open = self.is_debug_open();
        let ppu_open = self.is_ppu_open();
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        let wake = self.core.pump_debug(debug_open, ppu_open, &mut view);
        if view.applied {
            let ctx = self.core.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
        wake
    }

    /// Live PPU follow while the core runs: images-only refresh each interval.
    pub(crate) fn maybe_live_ppu(&mut self) {
        let debug_open = self.is_debug_open();
        let ppu_open = self.is_ppu_open();
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        self.core.pump_live_debug(debug_open, ppu_open, &mut view);
    }

    /// Open the debugger (idempotent); the PPU viewer only for the toolbar button.
    /// Closing never resumes; `focus_ppu` opens-or-focuses the viewer.
    pub(crate) fn open_debug_windows(&mut self, opener: &dyn WindowOpener, focus_ppu: bool) {
        if let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Debug) {
            let position = self.debug_window_pos();
            self.aux.window_mut(id).expect("debug reservation").title =
                self.core.debugger_window_title();
            let bridge = std::sync::Arc::new(self.core.new_debug_bridge());
            let opened = self.create_aux_window(
                id,
                opener,
                crate::ports::AuxOpenParams::Debug { bridge, position },
            );
            if opened {
                if let Some(bridge) = self.debug_bridge_for_ppu() {
                    let snapshot = self.aux.debug_snapshot(self.is_ppu_open());
                    self.core.open_debug_drain(&bridge, snapshot);
                }
                self.poll_debug_flight();
            } else {
                log::error!("failed to open debugger window");
            }
        }
        if focus_ppu && !self.is_ppu_open() {
            if let Some(bridge) = self.debug_bridge_for_ppu() {
                let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Ppu) else {
                    return;
                };
                let position = self.ppu_window_pos();
                self.aux.window_mut(id).expect("PPU reservation").title = bridge.ppu_chrome().title;
                let opened = self.create_aux_window(
                    id,
                    opener,
                    crate::ports::AuxOpenParams::Ppu { bridge, position },
                );
                if opened {
                    // Populate the image cache immediately: the viewer derives on Sync.
                    self.aux.ppu_sync();
                    // The viewer is open now, so re-read with images:
                    // the opening drain ran imageless.
                    self.refresh_open_debugger();
                } else {
                    log::error!("failed to open PPU viewer window");
                }
            }
        } else if focus_ppu {
            // Second and later presses: the viewer exists, so take
            // focus instead of no-op.
            self.aux.focus_kind(AuxKind::Ppu);
        }
    }

    /// Drain debugger requests and push one batched snapshot back.
    pub(crate) fn drain_debug_requests(&mut self) {
        let Some(bridge) = self.debug_bridge_for_ppu() else {
            return;
        };
        let snapshot = self.aux.debug_snapshot(self.is_ppu_open());
        // Contended bridge locks would wedge the UI thread:
        // all critical sections are short field copies.
        debug_assert!(snapshot.is_none() || self.aux.debug_locks_free());
        if self.core.take_debug_requests(&bridge, snapshot) {
            self.request_ppu_open();
        }
        self.poll_debug_flight();
    }

    /// Refresh the open viewer imagelessly, then poll. Bridgeless
    /// windows still enqueue so the chrome gate observes the drain.
    pub(crate) fn refresh_open_debugger(&mut self) {
        let bridge = self.debug_bridge_for_ppu();
        let snapshot = self.aux.debug_snapshot(self.is_ppu_open());
        self.core.refresh_debug_drain(bridge.as_deref(), snapshot);
        self.poll_debug_flight();
    }

    /// Poll the stored drain flight once; applies on completion.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        let pending = self.core.poll_debug(&mut view);
        if view.applied {
            let ctx = self.core.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
        pending
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self, ctx: &nerust_frontend_viewdefs::frame::FrameCtx) {
        self.sync_chrome(ctx, false);
        if self.core.chrome_drain_due(self.is_debug_open()) {
            self.refresh_open_debugger();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    /// Run separately under Xvfb: Tao/GTK3 initialization belongs to this thread.
    /// CI-only (ignored by default): needs a display, enabled in
    /// CI with `--run-ignored only` under `xvfb-run`.
    #[cfg(target_os = "linux")]
    #[ignore = "needs display; CI-only under xvfb"]
    #[test]
    fn native_common_windows_menu_effect_reservation_and_grouped_creation() {
        // libtest uses a new thread for every test, even with one test thread.
        // Isolate from the existing muda/GTK3 test's initialization thread.
        const CHILD: &str = "NERUST_TAO_WINDOW_NATIVE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "runtime::drain::tests::native_common_windows_menu_effect_reservation_and_grouped_creation",
                    "--exact", "--nocapture", "--test-threads=1", "--ignored",
                ])
                .env(CHILD, "1")
                .env("GDK_BACKEND", "x11")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .expect("isolated native window test");
            assert!(
                output.status.success(),
                "native child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use crate::{
            aux::focus::AuxKind,
            ports::{TaoWindowOpener, WindowOpener},
        };
        use gtk::prelude::*;
        use nerust_frontend_host::{
            commands::MenuCommand,
            menu::{MenuItem, MenuVisitor},
            window::{WidgetContents, WindowVisitor},
        };
        use tao::{event_loop::EventLoopBuilder, platform::unix::EventLoopBuilderExtUnix};

        struct FindDebugger(Option<MenuCommand>);
        impl nerust_frontend_host::commands::CommandVisitor for FindDebugger {
            fn command(&mut self, _: &nerust_frontend_host::commands::CommandDescriptor) {}
        }
        impl MenuVisitor for FindDebugger {
            fn begin_menubar(&mut self, _: &nerust_frontend_host::menu::Menubar, _: bool) {}
            fn begin_menu(&mut self, _: &nerust_frontend_host::menu::Submenu, _: bool) {}
            fn end_menu(&mut self, _: nerust_frontend_host::menu::MenuId) {}
            fn end_menubar(&mut self) {}
            fn item(&mut self, item: &MenuItem) {
                if item.command == Some(MenuCommand::Debugger) && item.enabled {
                    self.0 = item.command;
                }
            }
        }
        struct MainMenu(FindDebugger);
        impl WindowVisitor for MainMenu {
            fn visit_menu(&mut self, menu: &nerust_frontend_host::menu::Menubar) {
                menu.visit(None, &mut self.0);
            }
        }
        let event_loop = EventLoopBuilder::<crate::event::UserEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let opener = TaoWindowOpener::new(&event_loop);
        let mut host = TestHost::with_nes().host;
        host.ensure_window(&opener);
        assert!(host.window().is_some());
        let main_title = host.main_window().title.clone();
        assert!(gtk::Window::list_toplevels().iter().any(|widget| {
            widget
                .downcast_ref::<gtk::Window>()
                .and_then(|native| native.title())
                .as_deref()
                == Some(main_title.as_str())
        }));
        let ctx = host.frame_ctx();
        host.sync_menu_state(&ctx);
        let main = host.main_window().clone();
        let mut menu = MainMenu(FindDebugger(None));
        main.visit(&mut menu);
        let command = menu.0.0.expect("main_window.menu declares Debugger");
        host.on_menu_command(command, &opener);
        assert!(host.is_debug_open());
        let debug_id = host.aux.route_id(AuxKind::Debug).unwrap();
        let debug_window = host.aux.window(debug_id).unwrap().clone();
        assert!(matches!(
            debug_window.contents.contents,
            WidgetContents::Debug(_)
        ));
        assert_eq!(
            host.aux.request_open(AuxKind::Debug),
            nerust_frontend_host::aux::OpenRequest::Present(debug_id)
        );
        assert!(!host.aux.render_kind(AuxKind::Debug).unwrap().close);

        // The same native selection must use Window contents and title, even
        // when they are intentionally distinct from the original kind blueprint.
        for kind in [AuxKind::Ppu, AuxKind::History] {
            let nerust_frontend_host::aux::OpenRequest::Create(id) = host.aux.request_open(kind)
            else {
                panic!("expected reservation");
            };
            let mut session = if kind == AuxKind::History {
                host.prepare_widget_session(id)
            } else {
                None
            };
            if let Some(session) = session.as_mut() {
                use crate::aux::widgets::bindings::tests::{activate, controls};
                use wakui::control::{ControlEvent, ControlInput, ControlKind};
                host.report_host_error("native history before clear".into());
                host.refresh_widget_session(session);
                let filter = controls(&session.screen())
                    .into_iter()
                    .find(|c| matches!(c.kind, ControlKind::Choice { .. }))
                    .unwrap();
                session.dispatch(ControlEvent::new(filter.id, ControlInput::Select(3)));
                activate(session, "Clear History");
                assert!(session.mailbox().has_pending());
                assert!(!session.mailbox().close_requested());
            }
            host.aux.window_mut(id).unwrap().title = format!("host-owned {kind:?}");
            let window = host.aux.window(id).unwrap().clone();
            if kind == AuxKind::History {
                let mut mismatched = window.clone();
                mismatched.contents =
                    nerust_frontend_host::window::Window::auxiliary(AuxKind::Debug).contents;
                assert!(
                    opener
                        .create_aux(
                            mismatched,
                            crate::ports::AuxOpenParams::Controls {
                                session: {
                                    let mut other = TestHost::new().host;
                                    crate::aux::widgets::bindings::tests::prepare(&mut other, kind)
                                        .1
                                },
                            },
                        )
                        .is_none(),
                    "contents, not WindowId, must select the native path"
                );
            }
            let params = match kind {
                AuxKind::Ppu => crate::ports::AuxOpenParams::Ppu {
                    bridge: host.debug_bridge_for_ppu().unwrap(),
                    position: None,
                },
                AuxKind::History => crate::ports::AuxOpenParams::Controls {
                    session: session.unwrap(),
                },
                _ => unreachable!(),
            };
            let handle = opener
                .create_aux(window.clone(), params)
                .expect("native contents path");
            assert_eq!(handle.kind(), kind);
            assert!(host.aux.complete_open(id, handle).is_ok());
            let outcome = host.aux.render_kind(kind).unwrap();
            assert!(!outcome.close);
            assert!(gtk::Window::list_toplevels().iter().any(|widget| {
                widget
                    .downcast_ref::<gtk::Window>()
                    .and_then(|native| native.title())
                    .as_deref()
                    == Some(window.title.as_str())
            }));
            if kind == AuxKind::History {
                use crate::aux::widgets::bindings::tests::controls;
                use wakui::control::ControlKind;
                assert!(outcome.submitted);
                assert_eq!(outcome.id, Some(id));
                assert!(!host.finish_widget_window(id, false));
                assert_eq!(host.aux.route_id(kind), Some(id));
                let WidgetContents::Controls(screen) =
                    &host.aux.window(id).unwrap().contents.contents
                else {
                    panic!("generic history scene")
                };
                assert!(controls(screen).iter().any(|c| matches!(
                    c.kind,
                    ControlKind::Choice {
                        selected: Some(3),
                        ..
                    }
                )));
                assert!(!controls(screen).iter().any(|c| matches!(
                    &c.kind,
                    ControlKind::Note { text } if text.contains("native history before clear")
                )));
                host.report_host_error("native history after clear".into());
                host.aux.refresh_widget(id, &mut host.core);
                let WidgetContents::Controls(screen) =
                    &host.aux.window(id).unwrap().contents.contents
                else {
                    unreachable!()
                };
                assert!(controls(screen).iter().any(|c| matches!(
                    &c.kind,
                    ControlKind::Note { text } if text.contains("native history after clear")
                )));
                let mailbox = host.aux.windows.mailbox(id).unwrap();
                assert!(!mailbox.has_pending());
                mailbox.request_close();
                mailbox.invalidate();
                host.aux.refresh_widget(id, &mut host.core);
                assert!(host.aux.render_kind(kind).unwrap().close);
                assert!(!mailbox.take_invalidated());
            }
        }
        // Settings reserves a real screen before native creation, then reuses
        // that same single controller for iced boot, save staging and close.
        host.on_menu_command(MenuCommand::Settings, &opener);
        let settings_id = host.aux.route_id(AuxKind::Settings).unwrap();
        let settings = host.aux.window(settings_id).unwrap().clone();
        assert!(matches!(
            settings.contents.contents,
            WidgetContents::Controls(_)
        ));
        assert!(!host.aux.render_kind(AuxKind::Settings).unwrap().close);
        assert!(gtk::Window::list_toplevels().iter().any(|widget| {
            widget
                .downcast_ref::<gtk::Window>()
                .and_then(|native| native.title())
                .as_deref()
                == Some(settings.title.as_str())
        }));
        assert!(!host.close_settings_and_maybe_rebuild());
        assert!(host.aux.window(settings_id).is_none());
        host.on_menu_command(MenuCommand::Settings, &opener);
        let reopened = host.aux.route_id(AuxKind::Settings).unwrap();
        assert_ne!(settings_id, reopened);
        assert!(matches!(
            host.aux.window(reopened).unwrap().contents.contents,
            WidgetContents::Controls(_)
        ));
        assert!(!host.aux.render_kind(AuxKind::Settings).unwrap().close);
        host.prepare_close();
    }

    #[test]
    fn apply_drain_output_without_windows_syncs_chrome() {
        let support = TestHost::with_nes();
        let mut host = support.host;
        host.load_test_rom();
        let stub = crate::testkit::StubAux::new(crate::aux::focus::AuxKind::Debug);
        let applied = stub.applied_drain.clone();
        host.aux.insert(Box::new(stub));
        host.refresh_open_debugger();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while host.poll_debug_flight() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(applied.get());
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn drain_and_live_stay_quiet_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.take_debug_open_request());
        assert!(!host.take_ppu_open_request());
        assert!(!host.live_follow_active());
        host.live_tick();
        assert!(!host.poll_debug_flight());
        // No debugger open: the drain records pause truth and stops.
        host.drain_debug_requests();
        host.refresh_open_debugger();
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn tick_debug_pump_stays_idle_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        host.live_tick();
        host.maybe_live_ppu();
        assert!(!host.live_follow_active());
        let wake = host.tick_debug_pump();
        assert!(wake.live_tick.is_none());
        assert!(!wake.flight_pending);
        assert!(wake.osd_deadline.is_none());
    }

    #[test]
    fn debugger_open_fails_closed_without_drain() {
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, false);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!host.is_debug_open());
        // Failed open runs no drain, so the focus gate stays unset.
        assert_eq!(host.last_drain_paused(), None);
    }

    #[test]
    fn pending_debugger_open_skips_native_creation_and_drain() {
        use super::{AuxKind, OpenRequest};
        use crate::test_opener::FakeOpener;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host.aux.request_open(AuxKind::Debug) else {
            panic!("expected reservation");
        };
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, true);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::Debug), Some(id));
        assert!(!host.is_ppu_open());
        assert_eq!(host.last_drain_paused(), None);
    }

    #[test]
    fn viewer_open_without_debugger_needs_no_bridge() {
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, true);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!host.is_ppu_open());
    }

    #[test]
    fn live_images_without_payload_stay_total() {
        let support = TestHost::new();
        let mut host = support.host;
        use nerust_frontend_viewdefs::application::DebugView;
        let mut view = super::NativeDebugView {
            aux: &mut host.aux,
            applied: false,
        };
        view.apply_live(crate::aux::bridge::AppliedLive {
            generation: 0,
            images: Vec::new(),
            running: true,
        });
        assert!(!host.live_follow_active());
    }

    #[test]
    fn sync_chrome_without_debugger_skips_drain() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        host.sync_debug_chrome(&ctx);
        assert_eq!(support.menu.updates().len(), 1);
        // No debugger open: no drain runs, so the gate stays unset.
        assert_eq!(host.last_drain_paused(), None);
    }

    use crate::testkit::StubAux;

    fn bridge_stub(open_ppu: bool, refresh: bool) -> StubAux {
        use std::sync::atomic::Ordering;
        let bridge = crate::aux::bridge::DebugBridge::new(
            Default::default(),
            Vec::new(),
            0,
            nerust_settings_core::model::language::AppLanguage::English,
        );
        if refresh {
            bridge.dispatch(crate::aux::debug::DebugMessage::RefreshPressed);
        }
        if open_ppu {
            bridge.open_ppu.store(true, Ordering::SeqCst);
        }
        let mut stub = StubAux::new(crate::aux::focus::AuxKind::Debug);
        stub.bridge = Some(std::sync::Arc::new(bridge));
        stub
    }

    #[test]
    fn empty_batch_with_ppu_flag_arms_viewer_open() {
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(bridge_stub(true, false)));
        host.drain_debug_requests();
        assert!(host.take_ppu_open_request());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn nonempty_batch_without_snapshot_records_truth() {
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(bridge_stub(false, true)));
        // Stub snapshot stays None, so the round trip never runs.
        assert!(host.aux.debug_locks_free());
        host.drain_debug_requests();
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert!(!host.take_ppu_open_request());
    }
}
