//! Debug orchestrator: owns the aux-window cluster plus in-flight async flights.
//! `drain.rs` routes through these narrow methods, never through `host.debug` fields.

use super::super::debug_windows::DebugWindows;

pub(crate) struct DrainBatch {
    pub epoch: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub actions: Vec<nerust_gui_shell::debug::DebugAction>,
    pub snapshot: crate::aux::bridge::DrainSnapshot,
}

/// Debug orchestrator: aux-window cluster plus the in-flight async flights.
pub(crate) struct DebugOrchestrator {
    windows: DebugWindows,
    drain_flight: Option<nerust_gui_shell::debug::DrainFlight>,
    live_flight: Option<nerust_gui_shell::debug::LiveImagesFlight>,
    pub(crate) queue: std::collections::VecDeque<DrainBatch>,
    pub(crate) drain_epoch: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl DebugOrchestrator {
    pub(super) fn new() -> Self {
        Self {
            windows: DebugWindows::new(),
            drain_flight: None,
            live_flight: None,
            queue: Default::default(),
            drain_epoch: None,
        }
    }

    pub(crate) fn windows(&self) -> &DebugWindows {
        &self.windows
    }

    pub(crate) fn windows_mut(&mut self) -> &mut DebugWindows {
        &mut self.windows
    }

    /// Take the stored drain flight for polling.
    pub(crate) fn take_drain_flight(&mut self) -> Option<nerust_gui_shell::debug::DrainFlight> {
        self.drain_flight.take()
    }

    /// Store a still-pending drain flight (supersedes any older one).
    pub(crate) fn store_drain_flight(&mut self, flight: nerust_gui_shell::debug::DrainFlight) {
        self.drain_flight = Some(flight);
    }

    /// Take the stored live-images flight for polling.
    pub(crate) fn take_live_flight(&mut self) -> Option<nerust_gui_shell::debug::LiveImagesFlight> {
        self.live_flight.take()
    }

    /// Store a still-pending live-images flight.
    pub(crate) fn store_live_flight(&mut self, flight: nerust_gui_shell::debug::LiveImagesFlight) {
        self.live_flight = Some(flight);
    }

    pub(crate) fn live_flight_pending(&self) -> bool {
        self.live_flight.is_some()
    }

    pub(crate) fn drain_pending(&self) -> bool {
        self.drain_flight.is_some() || !self.queue.is_empty()
    }

    /// Drop in-flight async fetches (close teardown; handles live in the registry).
    pub(crate) fn close_flights(&mut self) {
        self.drain_flight.take();
        self.live_flight.take();
        self.queue.clear();
        self.drain_epoch = None;
    }
}

/// Narrow debug-batch port: debugger batch take (`None` when closed).
impl super::HostState {
    pub(crate) fn take_debug_batch(
        &mut self,
    ) -> Option<(bool, Vec<nerust_gui_shell::debug::DebugAction>)> {
        self.aux.debug_take_requests()
    }

    /// Live-tick interest for the parked tick thread (narrow port over the orchestrator).
    pub(crate) fn set_live_wanted(&mut self, wanted: bool) {
        self.debug.windows_mut().set_live_wanted(wanted);
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    #[test]
    fn debug_orchestrator_flights_start_empty_and_close_clean() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.orchestrator_mut().take_drain_flight().is_none());
        assert!(host.orchestrator_mut().take_live_flight().is_none());
        assert!(!host.live_flight_pending());
        host.orchestrator_mut().close_flights();
        assert!(!host.live_flight_pending());
    }
}
