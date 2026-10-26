//! Debug orchestrator: owns the aux-window cluster plus in-flight async flights.
//! Native windows stay here; FIFO and epoch-bound polling live in the shared host.

use super::super::debug_windows::DebugWindows;

pub(crate) type DrainBatch =
    nerust_frontend_host::drain::DrainBatch<crate::aux::bridge::DrainSnapshot>;

/// Debug orchestrator: aux-window cluster plus the in-flight async flights.
pub(crate) struct DebugOrchestrator {
    windows: DebugWindows,
    pub(crate) drain: nerust_frontend_host::drain::DrainQueue<crate::aux::bridge::DrainSnapshot>,
    pub(crate) live_flight: Option<nerust_gui_shell::debug::LiveImagesFlight>,
}

impl DebugOrchestrator {
    pub(super) fn new() -> Self {
        Self {
            windows: DebugWindows::new(),
            drain: Default::default(),
            live_flight: None,
        }
    }

    pub(crate) fn windows(&self) -> &DebugWindows {
        &self.windows
    }

    pub(crate) fn windows_mut(&mut self) -> &mut DebugWindows {
        &mut self.windows
    }

    /// Take the stored live-images flight for polling.
    #[cfg(test)]
    pub(crate) fn take_live_flight(&mut self) -> Option<nerust_gui_shell::debug::LiveImagesFlight> {
        self.live_flight.take()
    }

    pub(crate) fn live_flight_pending(&self) -> bool {
        self.live_flight.is_some()
    }

    pub(crate) fn drain_pending(&self) -> bool {
        self.drain.pending()
    }

    /// Drop in-flight async fetches (close teardown; handles live in the registry).
    pub(crate) fn close_flights(&mut self) {
        self.drain.clear();
        self.live_flight.take();
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
        assert!(!host.debug.drain.flight_pending());
        assert!(host.orchestrator_mut().take_live_flight().is_none());
        assert!(!host.live_flight_pending());
        host.orchestrator_mut().close_flights();
        assert!(!host.live_flight_pending());
    }
}
