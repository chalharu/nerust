//! Debugger and live-follow orchestration for `HostState`.
//! Drains, flights, aux-window management, and live PPU follow.

use std::{sync::Arc, time::Instant};

use tao::event_loop::EventLoopWindowTarget;

use nerust_gui_shell::{
    debug::{DebugDrainOutput, DrainPoll, LiveImagesPoll},
    session::LiveImages,
};
use nerust_gui_viewmodel::debugger as debug_vm;

use super::host::HostState;
use crate::aux::bridge::lock_recovered;
use crate::event::UserEvent;

impl HostState {
    pub(crate) fn take_debug_open_request(&mut self) -> bool {
        self.debug.take_debug_open_request()
    }

    pub(crate) fn take_ppu_open_request(&mut self) -> bool {
        self.debug.take_ppu_open_request()
    }

    /// Live follow is active: both aux windows open on a running
    /// loaded core. Single predicate for the tick, the wake backstop,
    /// and the redraw pump below.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.debug.debug_window.is_some()
            && self.debug.ppu_window.is_some()
            && self.session.loaded()
            && !self.session.paused()
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    /// Throttle-guarded inside; no-ops unless a running core has the
    /// viewer open.
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Live PPU follow while the core runs: images-only refresh each
    /// interval. Never a full drain (paused-gated inspect would blank
    /// the dump); paused cores need no periodic work; async flight.
    pub(crate) fn maybe_live_ppu(&mut self) {
        // Settle any in-flight fetch first (non-blocking): a flight
        // orphaned by pausing or closing still resolves harmlessly,
        // and polling here needs no throttle gate.
        if let Some(flight) = self.live_flight.as_mut() {
            match flight.poll() {
                LiveImagesPoll::Done(live) => {
                    self.live_flight = None;
                    self.apply_live_images(live);
                }
                LiveImagesPoll::Pending => return,
            }
        }
        if !self.live_follow_active() {
            return;
        }
        if !self.debug.live_due(Instant::now()) {
            return;
        }
        let mut flight = self.session.debug_live_images_begin(true);
        match flight.poll() {
            LiveImagesPoll::Done(live) => self.apply_live_images(live),
            LiveImagesPoll::Pending => {
                self.live_flight = Some(flight);
            }
        }
    }

    /// Patch a finished live fetch into the viewer. Images apply when
    /// newer regardless of pause (a paused frame is honest data); the
    /// running chrome only applies while still running, so a
    /// pause-in-flight never flips the status back to Running.
    fn apply_live_images(&mut self, live: LiveImages) {
        let Some((generation, images)) = live else {
            return;
        };
        let running = !self.session.paused();
        let Some(handle) = self.debug.debug_window.as_mut() else {
            return;
        };
        {
            let mut display = lock_recovered(&handle.bridge.display);
            if display.images_gen == generation {
                return;
            }
            display.images = images;
            display.images_gen = generation;
            if running {
                display.paused = false;
                display.status = nerust_gui_viewmodel::debugger::StatusMessage::Running;
            }
        }
        // Rebuild, not just repaint: the patch lands in bridge state,
        // and only a rebuild re-reads it into the widget tree.
        handle.sync_from_bridge();
        if let Some(ppu) = self.debug.ppu_window.as_mut() {
            ppu.sync_from_bridge();
        }
    }

    /// Open the debugger (idempotent), and the PPU viewer only for
    /// the toolbar button. Closing never resumes. Opening drains
    /// `[Pause, Refresh]` through the shared shell drain (pause first);
    /// `focus_ppu` opens-or-focuses the viewer for the toolbar only.
    pub(crate) fn open_debug_windows(
        &mut self,
        event_loop: &EventLoopWindowTarget<UserEvent>,
        focus_ppu: bool,
    ) {
        use nerust_gui_shell::debug::DebugAction;
        if self.debug.debug_window.is_none() {
            let spaces: Vec<(nerust_core_traits::debugger::SpaceId, String, u32)> = self
                .session
                .debug_spaces()
                .into_iter()
                .map(|info| (info.id, info.name.to_string(), *info.range.start()))
                .collect();
            if spaces.is_empty() {
                log::warn!("debugger: no spaces (no core)");
            }
            let mem_addr = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
            let position = self.debug.debug_window_pos();
            let language = self.session.settings_snapshot().shared.general.language;
            match crate::aux::debug::DebugWindowHandle::new(
                debug_vm::DisplayCache::default(),
                spaces,
                mem_addr,
                language,
                event_loop,
                position,
            ) {
                Some(handle) => {
                    self.debug.debug_window = Some(handle);
                    self.run_debug_drain(&[DebugAction::Pause, DebugAction::Refresh]);
                }
                None => log::error!("failed to open debugger window"),
            }
        }
        if focus_ppu
            && self.debug.ppu_window.is_none()
            && let Some(main) = self.debug.debug_window.as_ref()
        {
            let position = self.debug.ppu_window_pos();
            match crate::aux::ppu::PpuWindowHandle::new(
                Arc::clone(&main.bridge),
                event_loop,
                position,
            ) {
                Some(handle) => {
                    self.debug.ppu_window = Some(handle);
                    // Populate the image cache immediately: the bridge
                    // already carries images, but the viewer derives
                    // its scaled cache only on Sync.
                    if let Some(ppu) = self.debug.ppu_window.as_mut() {
                        ppu.sync_from_bridge();
                    }
                    // The viewer is open now, so re-read with images:
                    // the opening drain ran imageless.
                    self.run_debug_drain(&[]);
                }
                None => log::error!("failed to open PPU viewer window"),
            }
        } else if focus_ppu && let Some(ppu) = self.debug.ppu_window.as_ref() {
            // Second and later presses: the viewer exists, so take
            // focus instead of no-op.
            ppu.shell.window.set_focus();
        }
    }

    /// Drain debugger requests and push one batched snapshot back.
    /// Execution requests run before the re-read so one drain both
    /// acts and refreshes.
    pub(crate) fn drain_debug_requests(&mut self) {
        self.poll_debug_flight();
        let Some((open_ppu, requests)) = self.debug.take_debug_batch() else {
            return;
        };
        if open_ppu {
            self.debug.request_ppu_open();
        }
        if requests.is_empty() {
            return;
        }
        self.run_debug_drain(&requests);
    }

    /// Run one action batch through an async shell-drain flight.
    /// Empty batches are pure re-reads. Sends only; the event loop
    /// polls, so the UI thread never blocks.
    pub(crate) fn run_debug_drain(&mut self, requests: &[nerust_gui_shell::debug::DebugAction]) {
        use nerust_gui_shell::debug::DebugDrainInput;
        let (space, mem_addr, dis_addr, watch, freeze, reg_target, prev_rows, need_images) =
            match self.debug.debug_window.as_ref() {
                Some(handle) => {
                    let nav = lock_recovered(&handle.bridge.nav);
                    let mem_addr = nav.mem_addr();
                    let dis_addr = nav.dis_addr();
                    let prev = lock_recovered(&handle.bridge.display).dump_rows.clone();
                    drop(nav);
                    (
                        handle.bridge.selected_space(),
                        mem_addr,
                        dis_addr,
                        *lock_recovered(&handle.bridge.watch),
                        *lock_recovered(&handle.bridge.freeze),
                        *lock_recovered(&handle.bridge.reg_target),
                        prev,
                        self.debug.ppu_window.is_some(),
                    )
                }
                None => {
                    // No round-trip ran, so nothing could have moved:
                    // still record truth for the focus gate below.
                    self.debug.set_last_drain_paused(self.session.paused());
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
        // Supersede: a newer batch replaces the stored flight; the
        // FIFO command order keeps the newest view correct.
        let mut flight = self.session.debug_drain_begin(requests, &input);
        match flight.poll() {
            DrainPoll::Done(output) => self.apply_drain_output(*output),
            DrainPoll::Pending => {
                self.drain_flight = Some(flight);
            }
        }
    }

    /// Poll the stored drain flight once; applies on completion.
    /// Called from the event loop (request sites and the control-flow
    /// backstop) so every poll runs on the UI thread without ever
    /// blocking it. Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let mut flight = match self.drain_flight.take() {
            Some(flight) => flight,
            None => return false,
        };
        match flight.poll() {
            DrainPoll::Done(output) => {
                self.apply_drain_output(*output);
                false
            }
            DrainPoll::Pending => {
                self.drain_flight = Some(flight);
                true
            }
        }
    }

    /// Apply a finished drain: selection, running-sections merge, PPU
    /// follow, and the pause-truth agreement point (menu, title, and
    /// the focus gate below all follow the drained state).
    fn apply_drain_output(&mut self, output: DebugDrainOutput) {
        if let Some(handle) = self.debug.debug_window.as_mut() {
            if let Some(selected) = output.reselected {
                *lock_recovered(&handle.bridge.selected) = Some(selected);
            }
            // Running drains come back section-degenerate (paused
            // gate): restore the last paused rows so Resume keeps the
            // inspectable state instead of blanking it.
            let mut display = output.display;
            {
                // Images excluded: `restore_paused_sections` only reads
                // the six small paused fields, so dragging up to 512KB
                // of RGBA through every drain is pure waste.
                let old = lock_recovered(&handle.bridge.display).clone_without_images();
                display.restore_paused_sections(&old);
            }
            handle.set_display(display);
        }
        // PPU viewer follows the same drain (one scale per drain).
        if let Some(ppu) = self.debug.ppu_window.as_mut() {
            ppu.sync_from_bridge();
        }
        // The drain is the single pause-truth agreement point: the
        // toolbar label follows the drained state, so the menu and the
        // title sync here too instead of drifting apart (the reported
        // "two Pause statuses" bug).
        self.sync_menu_state();
        self.refresh_window_title();
        // Post-drain truth (the drain may itself pause/resume): focus
        // flips compare against this to skip redundant emu
        // round-trips (see `sync_debug_chrome`).
        self.debug.set_last_drain_paused(self.session.paused());
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; the
    /// emu snapshot only runs when pause state moved, so focus bursts
    /// never re-hit the thread.
    pub(crate) fn sync_debug_chrome(&mut self) {
        self.sync_menu_state();
        self.refresh_window_title();
        if chrome_drain_due(
            self.debug.debug_window.is_some(),
            self.debug.last_drain_paused(),
            self.session.paused(),
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
}
