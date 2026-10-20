use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::model::input::ShortcutAction;

use crate::session::SessionError;

/// Re-exported from the settings domain home; frontends keep importing it here.
pub use super::settings_apply::SettingsResult;

/// Pause control: the only session operations the modal auto-pause path needs.
/// Kept separate so callers depend on three methods, not the whole session surface.
pub trait PauseControl {
    fn pause(&mut self);
    fn resume(&mut self);
    fn toggle_pause(&mut self);
}

/// Save/load/delete leg of slot control, split from navigation.
/// Test fakes and frontends depend only on the half they use.
pub trait SlotIo {
    fn load_slot(&mut self, slot_id: u64) -> bool;
    fn save_slot(&mut self, slot_id: u64);
    fn delete_slot(&mut self, slot_id: u64);
    fn save_active_slot(&mut self) -> bool;
    fn load_active_slot(&mut self) -> bool;
}

/// Navigation leg of slot control: selection without I/O.
pub trait SlotNav {
    fn select_slot(&mut self, slot_id: u64);
    fn select_next_slot(&mut self);
    fn select_previous_slot(&mut self);
    fn create_slot(&mut self);
}

/// Reset leg: single method so pause/slot/reset vary independently.
pub trait ResetControl {
    fn reset(&mut self);
}

/// Session-level operations: teardown and settings application.
pub trait SessionControl {
    /// Unload the current ROM, flushing mapper saves.
    fn unload(&mut self);
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError>;
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError>;
}

/// Dispatch one schema session action; chrome-only actions return `false`
/// so frontends route them elsewhere.
pub fn dispatch_session_action<T>(target: &mut T, action: ShortcutAction) -> bool
where
    T: PauseControl + SlotIo + SlotNav + ResetControl + ?Sized,
{
    match action {
        ShortcutAction::TogglePause => {
            target.toggle_pause();
            true
        }
        ShortcutAction::SaveActiveSlot => target.save_active_slot(),
        ShortcutAction::SelectNextSlot => {
            target.select_next_slot();
            true
        }
        ShortcutAction::SelectPreviousSlot => {
            target.select_previous_slot();
            true
        }
        ShortcutAction::LoadActiveSlot => target.load_active_slot(),
        ShortcutAction::Reset => {
            target.reset();
            true
        }
        ShortcutAction::ToggleFullscreen | ShortcutAction::OpenDebugger => false,
    }
}

/// Direct narrow impls for the live session; new code depends on these.
impl PauseControl for super::SessionHandle {
    fn pause(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::Pause);
    }
    fn resume(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::Resume);
    }
    fn toggle_pause(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::TogglePause);
    }
}

/// Direct narrow impls for the live session; new code depends on these.
impl SlotIo for super::SessionHandle {
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.run_command(super::commands::SessionCommand::LoadSlot(slot_id))
            .map(|outcome| outcome.executed)
            .unwrap_or(false)
    }
    fn save_slot(&mut self, slot_id: u64) {
        let _ = self.run_command(super::commands::SessionCommand::SaveSlot(slot_id));
    }
    fn delete_slot(&mut self, slot_id: u64) {
        let _ = self.run_command(super::commands::SessionCommand::DeleteSlot(slot_id));
    }
    fn save_active_slot(&mut self) -> bool {
        self.run_command(super::commands::SessionCommand::SaveActiveSlotOrNew)
            .map(|outcome| outcome.executed)
            .unwrap_or(false)
    }
    fn load_active_slot(&mut self) -> bool {
        self.run_command(super::commands::SessionCommand::LoadActiveSlot)
            .map(|outcome| outcome.executed)
            .unwrap_or(false)
    }
}

/// Direct narrow impls for the live session; new code depends on these.
impl SlotNav for super::SessionHandle {
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self.run_command(super::commands::SessionCommand::SelectActiveSlot(slot_id));
    }
    fn select_next_slot(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::SelectPreviousSlot);
    }
    fn create_slot(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::CreateSlot);
    }
}

/// Direct narrow impl for the live session; new code depends on this.
impl ResetControl for super::SessionHandle {
    fn reset(&mut self) {
        let _ = self.run_command(super::commands::SessionCommand::Reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records dispatched calls. Every narrow-port method lands
    /// here so the routing test below pins each action exactly once.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<&'static str>,
    }

    impl PauseControl for Recorder {
        fn pause(&mut self) {
            self.calls.push("pause");
        }
        fn resume(&mut self) {
            self.calls.push("resume");
        }
        fn toggle_pause(&mut self) {
            self.calls.push("toggle_pause");
        }
    }

    impl SlotIo for Recorder {
        fn save_active_slot(&mut self) -> bool {
            self.calls.push("save_active_slot");
            true
        }
        fn load_active_slot(&mut self) -> bool {
            self.calls.push("load_active_slot");
            true
        }
        fn load_slot(&mut self, _slot_id: u64) -> bool {
            self.calls.push("load_slot");
            true
        }
        fn save_slot(&mut self, _slot_id: u64) {
            self.calls.push("save_slot");
        }
        fn delete_slot(&mut self, _slot_id: u64) {
            self.calls.push("delete_slot");
        }
    }

    impl SlotNav for Recorder {
        fn select_next_slot(&mut self) {
            self.calls.push("select_next_slot");
        }
        fn select_previous_slot(&mut self) {
            self.calls.push("select_previous_slot");
        }
        fn select_slot(&mut self, _slot_id: u64) {
            self.calls.push("select_slot");
        }
        fn create_slot(&mut self) {
            self.calls.push("create_slot");
        }
    }

    impl ResetControl for Recorder {
        fn reset(&mut self) {
            self.calls.push("reset");
        }
    }

    impl SessionControl for Recorder {
        fn unload(&mut self) {
            self.calls.push("unload");
        }
        fn apply_settings(
            &mut self,
            _settings: SettingsSnapshot,
        ) -> Result<SettingsResult, SessionError> {
            unimplemented!("routing test never applies settings")
        }
        fn set_fullscreen_default(
            &mut self,
            _fullscreen: bool,
        ) -> Result<SettingsResult, SessionError> {
            unimplemented!("routing test never touches fullscreen")
        }
    }

    #[test]
    fn dispatch_routes_every_session_action_once() {
        let mut recorder = Recorder::default();
        for (action, name) in [
            (ShortcutAction::TogglePause, "toggle_pause"),
            (ShortcutAction::SaveActiveSlot, "save_active_slot"),
            (ShortcutAction::SelectNextSlot, "select_next_slot"),
            (ShortcutAction::SelectPreviousSlot, "select_previous_slot"),
            (ShortcutAction::LoadActiveSlot, "load_active_slot"),
            (ShortcutAction::Reset, "reset"),
        ] {
            assert!(
                dispatch_session_action(&mut recorder, action),
                "{action:?} runs as a session operation"
            );
            assert_eq!(recorder.calls.last(), Some(&name), "{action:?}");
        }
        assert_eq!(recorder.calls.len(), 6, "no action double-dispatches");
        // Chrome actions are not session operations: silence, no call, unhandled.
        assert!(!dispatch_session_action(
            &mut recorder,
            ShortcutAction::ToggleFullscreen
        ));
        assert!(!dispatch_session_action(
            &mut recorder,
            ShortcutAction::OpenDebugger
        ));
        assert_eq!(recorder.calls.len(), 6);
    }

    #[test]
    fn recorder_pins_each_slot_and_lifecycle_call() {
        // Direct narrow impls: no adapter stands between the recorder and its ports.
        let mut recorder = Recorder::default();
        recorder.pause();
        recorder.resume();
        {
            let narrow: &mut dyn SlotIo = &mut recorder;
            assert!(narrow.save_active_slot());
            assert!(narrow.load_active_slot());
            narrow.save_slot(1);
            narrow.load_slot(1);
            narrow.delete_slot(1);
        }
        {
            let narrow: &mut dyn SlotNav = &mut recorder;
            narrow.select_slot(1);
            narrow.create_slot();
        }
        {
            let narrow: &mut dyn ResetControl = &mut recorder;
            narrow.reset();
        }
        recorder.unload();
        for name in [
            "pause",
            "resume",
            "save_slot",
            "load_slot",
            "delete_slot",
            "select_slot",
            "create_slot",
            "unload",
        ] {
            assert!(recorder.calls.contains(&name), "{name}");
        }
    }
}

#[cfg(test)]
mod narrow_tests {
    use super::{PauseControl, ResetControl, SlotIo, SlotNav};

    #[test]
    fn session_speaks_narrow_ports_directly() {
        fn bound<T: PauseControl + SlotIo + SlotNav + ResetControl>() {}
        bound::<super::super::SessionHandle>();
    }
}

#[cfg(test)]
mod access_cov_tests {
    use crate::session::access::*;
    use crate::session::test_util::test_session;

    #[test]
    fn settings_result_defaults_are_clean() {
        let result = SettingsResult::default();
        assert!(!result.renderer_needs_rebuild);
        assert!(!result.fullscreen_default_changed);
        assert!(!result.scaling_changed);
    }

    #[test]
    fn live_narrow_ports_tolerate_coreless_session() {
        use std::sync::Arc;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = super::super::SessionHandle::new_ephemeral(
            crate::session::test_util::test_session().capabilities,
            registry,
            audio,
        );
        session.pause();
        session.resume();
        session.toggle_pause();
        assert!(!session.load_slot(1));
        session.save_slot(1);
        session.delete_slot(1);
        assert!(!session.save_active_slot());
        assert!(!session.load_active_slot());
        session.select_slot(1);
        session.select_next_slot();
        session.select_previous_slot();
        session.create_slot();
        session.reset();
    }

    #[test]
    fn live_narrow_ports_run_with_core() {
        let mut session = test_session();
        session.pause();
        session.resume();
        session.toggle_pause();
        session.save_slot(1);
        session.delete_slot(1);
        session.select_slot(1);
        session.select_next_slot();
        session.select_previous_slot();
        session.create_slot();
        session.reset();
        let _ = session.save_active_slot();
        let _ = session.load_active_slot();
        let _ = session.load_slot(1);
    }

    #[test]
    fn narrow_impls_compose_without_adapter() {
        struct Probe {
            calls: Vec<&'static str>,
        }
        impl SlotIo for Probe {
            fn save_active_slot(&mut self) -> bool {
                self.calls.push("save_active_slot");
                true
            }
            fn load_active_slot(&mut self) -> bool {
                self.calls.push("load_active_slot");
                true
            }
            fn load_slot(&mut self, _: u64) -> bool {
                self.calls.push("load_slot");
                true
            }
            fn save_slot(&mut self, _: u64) {
                self.calls.push("save_slot");
            }
            fn delete_slot(&mut self, _: u64) {
                self.calls.push("delete_slot");
            }
        }
        impl SlotNav for Probe {
            fn select_next_slot(&mut self) {
                self.calls.push("select_next_slot");
            }
            fn select_previous_slot(&mut self) {
                self.calls.push("select_previous_slot");
            }
            fn select_slot(&mut self, _: u64) {
                self.calls.push("select_slot");
            }
            fn create_slot(&mut self) {
                self.calls.push("create_slot");
            }
        }
        impl ResetControl for Probe {
            fn reset(&mut self) {
                self.calls.push("reset");
            }
        }
        let mut probe = Probe { calls: vec![] };
        {
            let narrow: &mut dyn SlotIo = &mut probe;
            narrow.save_slot(9);
            assert!(narrow.load_slot(9));
            assert!(narrow.save_active_slot());
            assert!(narrow.load_active_slot());
            narrow.delete_slot(9);
        }
        {
            let narrow: &mut dyn SlotNav = &mut probe;
            narrow.select_slot(9);
            narrow.select_next_slot();
            narrow.select_previous_slot();
            narrow.create_slot();
        }
        {
            let narrow: &mut dyn ResetControl = &mut probe;
            narrow.reset();
        }
        for name in [
            "save_slot",
            "load_slot",
            "save_active_slot",
            "load_active_slot",
            "delete_slot",
            "select_slot",
            "select_next_slot",
            "select_previous_slot",
            "create_slot",
            "reset",
        ] {
            assert!(probe.calls.contains(&name), "{name}");
        }
    }
}
