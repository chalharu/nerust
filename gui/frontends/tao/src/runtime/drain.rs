//! Debugger and live-follow orchestration for `HostState`.
//! Drains run app-side per debug handle; native code pushes snapshots
//! into the aux windows and routes queued gestures into the session.

use nerust_frontend_host::{
    aux::OpenRequest,
    debugger::{DebugHandle, DebuggerScope},
};

use super::host::{HostState, WakeNeed};
use crate::aux::debug::views;
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

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
        let ppu_open = self.is_ppu_open();
        if self.core.debug_reset_if_replaced(handle) {
            // Stale inputs belonged to the retired epoch; the app rebuilt
            // the session, so drop queued gestures and resync everything.
            let _ = self.aux.drain_debug_queue();
        }
        for message in self.aux.drain_debug_queue() {
            let effects = self.core.debug_gesture(handle, vec![], message, ppu_open);
            if effects.open_ppu {
                self.core.request_ppu_open();
            }
        }
        let wake = self.core.debug_poll(handle, ppu_open);
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app(handle);
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
        let ppu_open = self.is_ppu_open();
        self.core.debug_live_tick(handle, ppu_open);
        let _ = self.core.debug_poll_live(handle, true, ppu_open);
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app(handle);
    }

    /// Push fresh pictures when the app generation moved.
    pub(super) fn sync_ppu_from_app(&mut self, handle: DebugHandle) {
        if !self.is_ppu_open() {
            return;
        }
        let generation = self.core.debug_generation(handle);
        if generation == self.ppu_generation {
            return;
        }
        self.ppu_generation = generation;
        let pictures = self.core.debug_pictures(handle);
        let panels = self.core.debug_panels(handle);
        let has_panels = panels.is_some();
        let chrome = self.core.ppu_chrome();
        let payload = crate::aux::ppu::PpuPictures {
            pictures,
            panels: panels.unwrap_or_default(),
            has_panels,
            chrome,
            generation,
        };
        self.aux.push_ppu_pictures(payload);
        // Hover lines queued while the viewer moved resolve app-side.
        self.drain_ppu_hover(handle);
    }

    /// Resolve queued hover coordinates into text on the viewer.
    pub(super) fn drain_ppu_hover(&mut self, handle: DebugHandle) {
        let hovered = self.aux.drain_ppu_hover();
        for at in hovered {
            let line = self
                .core
                .debug_hover_line(handle, &at.label, at.width, at.height, at.x, at.y);
            self.aux.push_ppu_hover(line);
        }
    }

    /// Open the debugger (idempotent); the PPU viewer only for the toolbar button.
    /// Closing never resumes; `focus_ppu` opens-or-focuses the viewer.
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
            if let Some(handle) = self.debug_handle {
                let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Ppu) else {
                    return;
                };
                let position = self.core.ppu_window_pos();
                self.aux.window_mut(id).expect("PPU reservation").title =
                    self.core.ppu_chrome().title.clone();
                let opened = self.create_aux_window(
                    id,
                    opener,
                    crate::ports::AuxOpenParams::Ppu { position },
                );
                if opened {
                    // Populate the image cache immediately: the viewer derives on Sync.
                    self.sync_ppu_from_app(handle);
                    // The viewer is open now, so refresh with images:
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

    /// Poll the stored drain flight once; push the fresh snapshot.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let Some(handle) = self.debug_handle else {
            return false;
        };
        let wake = self.core.debug_poll(handle, self.is_ppu_open());
        self.push_debug_snapshot(handle);
        self.sync_ppu_from_app(handle);
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
