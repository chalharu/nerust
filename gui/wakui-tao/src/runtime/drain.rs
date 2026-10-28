//! Debugger and live-follow orchestration for `HostState`.
//! Drains run app-side per debug handle; native code pushes snapshots
//! into the aux windows and routes queued gestures into the session.
//! The PPU viewer is a generic widget window (ControlScreen with Image
//! controls); its pictures refresh through `widget_refresh`, hover through
//! `widget_dispatch`.

use wakui::aux::OpenRequest;
use wakui::debugger::{DebugHandle, DebuggerScope};

use super::host::{HostState, WakeNeed};
use crate::aux::debug::views;
use crate::aux::focus::AuxKind;
use crate::ports::{AuxOpenParams, WindowOpener};

impl HostState {
    /// App-side session behind the open debugger/PPU windows, if any.
    #[cfg(test)]
    pub(super) fn debug_handle(&self) -> Option<DebugHandle> {
        self.debug_handle
    }

    /// Drop the shared session once both viewers are gone.
    pub(super) fn maybe_close_debug_session(&mut self) {
        if !self.is_debug_open()
            && !self.is_ppu_open()
            && let Some(handle) = self.debug_handle.take()
        {
            self.core.close_debug(handle);
        }
    }

    /// Live follow is active: both aux windows open on a running loaded core.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.core.tick_loaded()
            && !self.core.tick_paused()
            && self.is_debug_open()
            && self.is_ppu_open()
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Debug pump for the control-flow pass: gestures, poll, snapshot push.
    pub(super) fn tick_debug_pump(&mut self) -> WakeNeed {
        let Some(handle) = self.debug_handle else {
            return WakeNeed::default();
        };
        let need_images = self.is_ppu_open();
        if self.core.debug_reset_if_replaced(handle) {
            // Stale inputs belonged to the retired epoch; the app rebuilt
            // the session, so drop queued gestures and resync everything.
            let _ = self.aux.drain_debug_queue();
        }
        for message in self.aux.drain_debug_queue() {
            let effects = self
                .core
                .debug_gesture(handle, vec![], message, need_images);
            if effects.open_ppu {
                self.core.request_ppu_open();
            }
        }
        let wake = self.core.debug_poll(handle, need_images);
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app();
        wake
    }

    /// Push the latest app snapshot into the open debug window.
    pub(super) fn push_debug_snapshot(&mut self, handle: DebugHandle) -> bool {
        let Some(snapshot) = self.core.debug_snapshot(handle) else {
            return false;
        };
        let view = views::snapshot_view(&snapshot);
        if !self.aux.push_debug_snapshot(&view) {
            return false;
        }
        self.sync_chrome(false);
        true
    }

    /// Live PPU follow while the core runs: images-only refresh each interval.
    pub(crate) fn maybe_live_ppu(&mut self) {
        let Some(handle) = self.debug_handle else {
            return;
        };
        if !self.live_follow_active() {
            return;
        }
        let need_images = self.is_ppu_open();
        self.core.debug_live_tick(handle, need_images);
        let _ = self.core.debug_poll_live(handle, true, need_images);
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app();
    }

    /// Refresh the generic PPU widget from the view-side session.
    /// The view definition owns pictures/panels/chrome; native only pushes
    /// the projected ControlScreen.
    pub(super) fn sync_ppu_from_app(&mut self) {
        let Some(id) = self.aux.route_id(AuxKind::Ppu) else {
            return;
        };
        self.core.widget_refresh(id);
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
    }

    /// Open the debugger (idempotent); the PPU viewer only for the toolbar button.
    /// Closing never resumes; `focus_ppu` opens-or-focuses the viewer.
    /// PPU opens as a generic widget window, like Settings/History.
    pub(crate) fn open_debug_windows(&mut self, opener: &dyn WindowOpener, focus_ppu: bool) {
        if let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Debug) {
            let position = self.core.debug_window_pos();
            let handle = match self.debug_handle {
                Some(handle) => handle,
                None => {
                    let handle = self.core.open_debug();
                    self.debug_handle = Some(handle);
                    handle
                }
            };
            self.core.debug_open_refresh(handle, self.is_ppu_open());
            self.aux.window_mut(id).expect("debug reservation").title =
                self.core.debugger_window_title();
            let opened =
                self.create_aux_window(id, opener, crate::ports::AuxOpenParams::Debug { position });
            if opened {
                self.poll_debug_flight();
            } else {
                log::error!("failed to open debugger window");
                // Failed open keeps no session: the viewer arm below
                // must observe no handle, like the old bridgeless path.
                self.core.close_debug(handle);
                if self.debug_handle == Some(handle) {
                    self.debug_handle = None;
                }
            }
        }
        if focus_ppu && !self.is_ppu_open() {
            // PPU needs a live debugger handle, like the old viewer path:
            // headless Debug failure must not open a bridgeless PPU.
            if self.debug_handle.is_none() {
                return;
            }
            let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Ppu) else {
                return;
            };
            if !self.prepare_widget(id) {
                return;
            }
            let (screen, mailbox) =
                match (self.core.widget_screen(id), self.core.widget_mailbox(id)) {
                    (Some(screen), Some(mailbox)) => (screen, mailbox),
                    _ => {
                        self.core.cancel_widget(&mut self.aux.windows, id);
                        return;
                    }
                };
            let opened = self.create_aux_window(
                id,
                opener,
                AuxOpenParams::Controls {
                    id,
                    screen: Box::new(screen),
                    mailbox,
                },
            );
            if opened {
                // Populate immediately so the viewer never sits on PendingControls.
                self.sync_ppu_from_app();
                // The viewer is open now, so refresh with images:
                // the opening drain ran imageless.
                self.refresh_open_debugger();
            } else {
                log::error!("failed to open PPU viewer window");
            }
        } else if focus_ppu {
            // Second and later presses: the viewer exists, so take
            // focus instead of no-op.
            self.aux.focus_kind(AuxKind::Ppu);
        }
    }

    /// Poll the stored drain flight once; push the fresh snapshot.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let Some(handle) = self.debug_handle else {
            return false;
        };
        let wake = self.core.debug_poll(handle, self.is_ppu_open());
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app();
        wake.flight_pending
    }

    /// Drain-through entry kept for callers that only need a poll.
    pub(crate) fn drain_debug_requests(&mut self) {
        self.poll_debug_flight();
    }

    /// Refresh the open viewer, then poll. The app enqueues Pause+Refresh
    /// through the shared session; native code only pushes the result.
    pub(crate) fn refresh_open_debugger(&mut self) {
        let Some(handle) = self.debug_handle else {
            return;
        };
        self.core
            .debug_refresh(handle, DebuggerScope::All, self.is_ppu_open());
        self.poll_debug_flight();
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self) {
        self.sync_chrome(false);
        if self.core.chrome_drain_due(self.is_debug_open()) {
            self.refresh_open_debugger();
        }
    }
}
