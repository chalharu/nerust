//! Headless test double: canonical stub lives at `aux::registry::test_double`.
//! This module re-exports it for consumer test suites.

use crate::aux::domain::AuxNativeEvent;
use crate::aux::focus::AuxKind;
use crate::aux::registry::AuxController;

pub(crate) use crate::aux::registry::test_double::StubAux;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::debug::DebugMessage;

    #[test]
    fn stub_answers_only_its_own_kind() {
        let mut debug = StubAux::new(AuxKind::Debug);
        assert!(debug.as_widget().is_none());
        assert!(debug.widget_mailbox().is_none());
        assert_eq!(debug.drain_debug_queue().len(), 0);
        debug.debug_queue.push(DebugMessage::RefreshPressed);
        assert_eq!(debug.drain_debug_queue().len(), 1);

        let mut history = StubAux::new(AuxKind::History);
        assert!(history.as_widget().is_some());
        assert!(history.widget_mailbox().is_none());
        let mut settings = StubAux::new(AuxKind::Settings);
        assert!(settings.as_widget().is_some());
        assert!(settings.widget_mailbox().is_none());
    }

    #[test]
    fn stub_ports_default_to_closed_quiet() {
        use crate::aux::registry::AuxRegistry;

        let mut stub = StubAux::new(AuxKind::History);
        assert!(stub.drain_debug_queue().is_empty());
        assert!(stub.widget_mailbox().is_none());
        assert!(!stub.on_redraw().submitted);
        // Kind-gated queries: a history stub never answers debug ports.
        assert!(stub.as_widget().is_some());
        let mut registry = AuxRegistry::new();
        let id = registry.insert(Box::new(stub));
        assert!(registry.drain_widget_events(id).is_empty());
    }

    #[test]
    fn stub_pump_reports_close_and_clear_flags() {
        let mut history = StubAux::new(AuxKind::History);
        history.close = true;
        let mut host = crate::runtime::host::TestHost::new().host;
        let id = crate::aux::widgets::bindings::tests::prepare(&mut host, AuxKind::History);
        history.mailbox = crate::aux::widgets::bindings::tests::mailbox_of(&host, id).into();
        crate::aux::widgets::bindings::tests::activate(&mut host, id, "Clear History");
        assert!(
            crate::aux::widgets::bindings::tests::mailbox_of(&host, id).has_pending(),
            "activation stages a submission through the app"
        );
        let outcome = history.on_event(AuxNativeEvent::Focused(true));
        assert!(outcome.close);
        assert!(outcome.submitted);
    }
}
