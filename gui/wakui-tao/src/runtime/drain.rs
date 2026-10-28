//! Debugger drain orchestration for `HostState`.
//! The debugger is a generic widget window: drains run app-side per widget id,
//! native code pushes the projected screen and routes queued gestures.
//! Deferred aux opens (e.g. viewers requested from a widget gesture) are
//! drained wherever a window opener is available.

use wakui::debugger::DebuggerScope;

use super::host::HostState;
use crate::ports::WindowOpener;

impl<K: Copy + Eq + std::hash::Hash + 'static> HostState<K> {
    /// Live-follow truth for the turn driver: a loaded running core behind
    /// the open debugger widget.
    pub(crate) fn debug_widget_live_follow(&self) -> bool {
        self.core.tick_loaded() && !self.core.tick_paused() && self.core.debug_widget().is_some()
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self) {
        self.sync_chrome(false);
        if self.core.chrome_drain_due() {
            self.refresh_open_debugger();
        }
    }

    /// Debug pump for the control-flow pass: gestures, poll, snapshot push.
    pub(super) fn pump_debug(&mut self) {
        let Some(id) = self.core.debug_widget() else {
            return;
        };
        let Some(handle) = self.core.debug_handle_for(id) else {
            return;
        };
        if self.core.debug_reset_if_replaced(handle) {
            // Stale inputs belonged to the retired epoch; the app rebuilt
            // the session, so drop queued gestures and resync everything.
            let _ = self.aux.drain_widget_events(id);
        }
        for event in self.aux.drain_widget_events(id) {
            for effect in self.core.widget_dispatch(id, event) {
                self.apply_control_effect(id, effect);
            }
        }
        let _wake = self.core.debug_poll(handle);
        if self.core.chrome_drain_due() {
            self.core.debug_refresh(handle, DebuggerScope::All);
            let _ = self.core.debug_poll(handle);
        }
        self.core.widget_refresh(id);
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
        self.sync_chrome(false);
    }

    /// Live follow pulse (timer thread via `UserEvent::LiveTick`): images-only
    /// refresh while a loaded core runs behind the open debugger widget.
    pub(crate) fn live_tick(&mut self) {
        let Some(id) = self.core.debug_widget() else {
            return;
        };
        if !(self.core.tick_loaded() && !self.core.tick_paused()) {
            return;
        }
        let Some(handle) = self.core.debug_handle_for(id) else {
            return;
        };
        self.core.debug_live_tick(handle);
        let _ = self.core.debug_poll_live(handle, true);
        self.core.widget_refresh(id);
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
    }

    /// Refresh the open debugger widget, then poll. The app enqueues
    /// Pause+Refresh through the shared session; native code pushes the result.
    pub(crate) fn refresh_open_debugger(&mut self) {
        let Some(id) = self.core.debug_widget() else {
            return;
        };
        let Some(handle) = self.core.debug_handle_for(id) else {
            return;
        };
        self.core.debug_refresh(handle, DebuggerScope::All);
        let _ = self.core.debug_poll(handle);
        self.core.widget_refresh(id);
        if let Some(screen) = self.core.widget_screen(id) {
            self.aux.push_widget_snapshot(id, &screen);
        }
    }

    /// Open one aux window of this kind, idempotently; deferred requests
    /// drain through the same path wherever an opener is available.
    pub(crate) fn pump_deferred_opens(&mut self, opener: &dyn WindowOpener) {
        for kind in self.core.drain_open_requests() {
            self.open_aux_window(kind, opener);
        }
    }

    /// Focus-only drive shared by every aux path: arrival reports true,
    /// departure and non-focus events report false, without touching chrome.
    pub(crate) fn drive_aux_focus(
        &mut self,
        id: wakui::aux::AuxId,
        event: &tao::event::WindowEvent,
    ) -> bool {
        match event {
            tao::event::WindowEvent::Focused(focused) => self.core.drive_aux_focus(id, *focused),
            _ => false,
        }
    }
}
