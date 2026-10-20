//! Debugger and live-follow orchestration for `HostState`.
//! Drains, flights, aux-window management, and live PPU follow.

use std::time::Instant;

use nerust_gui_shell::{
    debug::{DebugDrainOutput, DrainPoll, LiveImagesPoll},
    session::LiveImages,
};
use nerust_gui_viewmodel::debugger as debug_vm;

use super::host::HostState;
use crate::aux::focus::AuxWindow;
use crate::ports::WindowOpener;

impl HostState {
    pub(crate) fn take_debug_open_request(&mut self) -> bool {
        self.orchestrator_mut()
            .windows_mut()
            .take_debug_open_request()
    }

    pub(crate) fn take_ppu_open_request(&mut self) -> bool {
        self.orchestrator_mut()
            .windows_mut()
            .take_ppu_open_request()
    }

    /// Live follow is active: both aux windows open on a running loaded core.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.is_debug_open()
            && self.is_ppu_open()
            && self.session().tick_metrics().loaded
            && !self.session().tick_metrics().paused
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Live PPU follow while the core runs: images-only refresh each interval.
    pub(crate) fn maybe_live_ppu(&mut self) {
        // Settle any in-flight fetch first (non-blocking).
        if let Some(mut flight) = self.orchestrator_mut().take_live_flight() {
            match flight.poll() {
                LiveImagesPoll::Done(live) => self.apply_live_images(live),
                LiveImagesPoll::Pending => {
                    self.orchestrator_mut().store_live_flight(flight);
                    return;
                }
            }
        }
        if !self.live_follow_active() {
            return;
        }
        if !self.live_due(Instant::now()) {
            return;
        }
        let mut flight = self.session().debug_live_images_begin(true);
        match flight.poll() {
            LiveImagesPoll::Done(live) => self.apply_live_images(live),
            LiveImagesPoll::Pending => {
                self.orchestrator_mut().store_live_flight(flight);
            }
        }
    }

    /// Patch a finished live fetch into the viewer: images land when newer.
    /// Bridge writes route through the handle narrow port.
    fn apply_live_images(&mut self, live: LiveImages) {
        let Some((generation, images)) = live else {
            return;
        };
        let running = !self.session().tick_metrics().paused;
        let had_debug = self
            .with_debug_window(|handle| {
                if handle.apply_live(generation, images, running) {
                    // Rebuild, not just repaint: only a rebuild re-reads bridge state.
                    handle.sync_from_bridge();
                }
            })
            .is_some();
        if had_debug {
            self.with_ppu_window(|ppu| {
                ppu.sync_from_bridge();
            });
        }
    }

    /// Open the debugger (idempotent); the PPU viewer only for the toolbar button.
    /// Closing never resumes; `focus_ppu` opens-or-focuses the viewer.
    pub(crate) fn open_debug_windows(&mut self, opener: &dyn WindowOpener, focus_ppu: bool) {
        use nerust_gui_shell::debug::DebugAction;
        if !self.is_debug_open() {
            let spaces: Vec<(nerust_core_traits::debugger::SpaceId, String, u32)> = self
                .session()
                .debug_spaces()
                .into_iter()
                .map(|info| (info.id, info.name.to_string(), *info.range.start()))
                .collect();
            if spaces.is_empty() {
                log::warn!("debugger: no spaces (no core)");
            }
            let mem_addr = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
            let position = self.debug_window_pos();
            let language = self.session().settings_snapshot().shared.general.language;
            match opener.create_debug(
                debug_vm::DisplayCache::default(),
                spaces,
                mem_addr,
                language,
                position,
            ) {
                Some(handle) => {
                    self.set_debug_window(handle);
                    self.run_debug_drain(&[DebugAction::Pause, DebugAction::Refresh]);
                }
                None => log::error!("failed to open debugger window"),
            }
        }
        if focus_ppu && !self.is_ppu_open() {
            if let Some(bridge) = self.debug_bridge_for_ppu() {
                let position = self.ppu_window_pos();
                match opener.create_ppu(&bridge, position) {
                    Some(handle) => {
                        self.set_ppu_window(handle);
                        // Populate the image cache immediately: the viewer derives on Sync.
                        self.with_ppu_window(|ppu| {
                            ppu.sync_from_bridge();
                        });
                        // The viewer is open now, so re-read with images:
                        // the opening drain ran imageless.
                        self.run_debug_drain(&[]);
                    }
                    None => log::error!("failed to open PPU viewer window"),
                }
            }
        } else if focus_ppu {
            // Second and later presses: the viewer exists, so take
            // focus instead of no-op.
            self.with_ppu_window(|ppu| {
                ppu.focus_window();
            });
        }
    }

    /// Drain debugger requests and push one batched snapshot back.
    pub(crate) fn drain_debug_requests(&mut self) {
        self.poll_debug_flight();
        let Some((open_ppu, requests)) = self.take_debug_batch() else {
            return;
        };
        if open_ppu {
            self.request_ppu_open();
        }
        if requests.is_empty() {
            return;
        }
        self.run_debug_drain(&requests);
    }

    /// Run one action batch through an async shell-drain flight.
    /// Sends only; the event loop polls, so the UI thread never blocks.
    pub(crate) fn run_debug_drain(&mut self, requests: &[nerust_gui_shell::debug::DebugAction]) {
        use crate::aux::bridge::BridgeLock;
        use nerust_gui_shell::debug::DebugDrainInput;
        let ppu_open = self.is_ppu_open();
        let (space, mem_addr, dis_addr, watch, freeze, reg_target, prev_rows, need_images) =
            match self.with_debug_window(|handle| {
                let lock: &dyn BridgeLock = handle.bridge_lock();
                // Contended bridge locks would wedge the UI thread:
                // all critical sections are short field copies.
                debug_assert!(lock.try_lock_audit().is_empty());
                lock.drain_snapshot(ppu_open)
            }) {
                Some(snapshot) => snapshot,
                None => {
                    // No round-trip ran, so nothing could have moved:
                    // still record truth for the focus gate below.
                    self.set_last_drain_paused(self.session().tick_metrics().paused);
                    return;
                }
            };
        let input = DebugDrainInput {
            space,
            mem_addr,
            dis_addr,
            watch,
            freeze,
            reg_target,
            prev_dump_rows: &prev_rows,
            need_images,
        };
        // Supersede: a newer batch replaces the stored flight.
        let mut flight = self.session().debug_drain_begin(requests, &input);
        match flight.poll() {
            DrainPoll::Done(output) => self.apply_drain_output(*output),
            DrainPoll::Pending => {
                self.orchestrator_mut().store_drain_flight(flight);
            }
        }
    }

    /// Poll the stored drain flight once; applies on completion.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let mut flight = match self.orchestrator_mut().take_drain_flight() {
            Some(flight) => flight,
            None => return false,
        };
        match flight.poll() {
            DrainPoll::Done(output) => {
                self.apply_drain_output(*output);
                false
            }
            DrainPoll::Pending => {
                self.orchestrator_mut().store_drain_flight(flight);
                true
            }
        }
    }

    /// Apply a finished drain: selection, running-sections merge, PPU follow.
    fn apply_drain_output(&mut self, output: DebugDrainOutput) {
        let reselected = output.reselected;
        let display = output.display;
        // Selection write plus paused-section restore live in the bridge narrow port.
        self.with_debug_window(|handle| {
            handle.apply_drain_display(display, reselected);
            handle.sync_from_bridge();
        });
        // PPU viewer follows the same drain (one scale per drain).
        self.with_ppu_window(|ppu| {
            ppu.sync_from_bridge();
        });
        // The drain is the single pause-truth agreement point: menu and title sync here.
        self.sync_menu_state();
        self.refresh_window_title();
        // Post-drain truth: focus flips compare against this to skip redundant round-trips.
        self.set_last_drain_paused(self.session().tick_metrics().paused);
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self) {
        self.sync_chrome(false);
        if chrome_drain_due(
            self.is_debug_open(),
            self.last_drain_paused(),
            self.session().tick_metrics().paused,
        ) {
            self.run_debug_drain(&[]);
        }
    }
}

fn chrome_drain_due(debug_open: bool, last_drain_paused: Option<bool>, paused: bool) -> bool {
    debug_open && last_drain_paused != Some(paused)
}

#[cfg(test)]
mod tests {
    use super::chrome_drain_due;
    use crate::runtime::host::TestHost;

    #[test]
    fn focus_chrome_drains_only_on_pause_transitions() {
        // Closed debugger: never worth a round-trip.
        assert!(!chrome_drain_due(false, None, false));
        assert!(!chrome_drain_due(false, Some(true), false));
        // First focus sync establishes the baseline.
        assert!(chrome_drain_due(true, None, true));
        assert!(chrome_drain_due(true, None, false));
        // Steady state: focus flips stay local-only.
        assert!(!chrome_drain_due(true, Some(true), true));
        assert!(!chrome_drain_due(true, Some(false), false));
        // A pause flip behind the toolbar's back (F9, auto-pause)
        // re-arms exactly one drain.
        assert!(chrome_drain_due(true, Some(true), false));
        assert!(chrome_drain_due(true, Some(false), true));
    }

    #[test]
    fn apply_drain_output_without_windows_syncs_chrome() {
        use nerust_gui_shell::debug::DebugDrainOutput;
        use nerust_gui_viewmodel::debugger::DisplayCache;
        let support = TestHost::new();
        let mut host = support.host;
        let output = DebugDrainOutput {
            display: DisplayCache::default(),
            reselected: Some((0x100, Some(0xAB))),
        };
        host.apply_drain_output(output);
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
        host.run_debug_drain(&[]);
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert!(support.menu.updates().is_empty());
    }
}
