//! Headless test double: canonical stub lives at `aux::registry::test_double`.
//! This module re-exports it for consumer test suites.

use std::rc::Rc;

use nerust_gui_shell::debug::DebugAction;

use crate::aux::domain::AuxNativeEvent;
use crate::aux::focus::AuxKind;
use crate::aux::registry::{AuxController, DebugDrainPort, LiveImagesPort, SettingsClosePort};

pub(crate) use crate::aux::registry::test_double::StubAux;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_answers_only_its_own_kind() {
        use nerust_gui_viewmodel::debugger::DisplayCache;
        let mut debug = StubAux::new(AuxKind::Debug);
        debug.debug_batch = vec![DebugAction::Refresh];
        assert!(debug.as_debug().is_some());
        assert!(debug.as_live().is_some());
        assert!(debug.as_settings().is_none());
        assert!(debug.as_history().is_none());
        assert!(debug.debug_snapshot_of(true).expect("debug snaps").ppu_open);
        let applied = Rc::clone(&debug.applied_drain);
        debug
            .as_debug()
            .expect("debug")
            .apply_drain(DisplayCache::default(), None);
        assert!(applied.get());

        let mut history = StubAux::new(AuxKind::History);
        assert!(history.as_debug().is_none());
        assert!(history.as_history().is_some());
        let mut settings = StubAux::new(AuxKind::Settings);
        assert!(settings.as_settings().is_some());
        assert!(settings.as_debug().is_none());
        assert!(settings.take_pending_settings().is_none());
    }

    #[test]
    fn stub_ports_default_to_closed_quiet() {
        use nerust_gui_viewmodel::debugger::DisplayCache;
        let mut stub = StubAux::new(AuxKind::History);
        assert!(DebugDrainPort::take_debug_requests(&mut stub).is_none());
        assert!(stub.debug_snapshot_of(false).is_none());
        assert!(!LiveImagesPort::apply_live(&mut stub, 1, Vec::new(), false));
        assert!(SettingsClosePort::take_pending_settings(&mut stub).is_none());
        assert!(!stub.take_history_clear());
        assert!(stub.shared_bridge().is_none());
        assert!(stub.debug_lock_audit().is_empty());
        // Kind-gated queries: a history stub never answers debug ports.
        assert!(stub.as_debug().is_none());
        assert!(stub.as_settings().is_none());
        assert!(stub.as_history().is_some());
        let _ = DisplayCache::default();
    }

    #[test]
    fn stub_pump_reports_close_and_clear_flags() {
        let mut history = StubAux::new(AuxKind::History);
        history.close = true;
        history.clear_history = true;
        let outcome = history.on_event(AuxNativeEvent::Focused(true));
        assert!(outcome.close);
        assert!(outcome.cleared);
    }
}
