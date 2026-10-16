use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::model::input::ShortcutAction;

use crate::session::SessionError;

/// Summary of a settings-apply operation, visible to frontends.
///
/// Replaces the full `SettingsApplyPlan` (which contains implementation
/// details like `session_rebuild_required`, etc.) with only the flags
/// that frontends actually act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsResult {
    /// The emulation core's renderer (GPU surface) needs to be recreated.
    pub renderer_needs_rebuild: bool,
    /// The fullscreen-default setting changed; frontend should sync the
    /// window's fullscreen state.
    pub fullscreen_default_changed: bool,
    /// The window-scaling mode changed; frontend should resize the window.
    pub scaling_changed: bool,
}

/// Pause control: the only session operations the modal auto-pause
/// path needs. Kept separate so callers (and test fakes) depend on
/// three methods, not the whole session surface.
pub trait PauseControl {
    fn pause(&mut self);
    fn resume(&mut self);
    fn toggle_pause(&mut self);
}

/// Save-slot control: slot CRUD plus reset.
pub trait SlotControl {
    fn save_active_slot(&mut self) -> bool;
    fn load_active_slot(&mut self) -> bool;
    fn select_next_slot(&mut self);
    fn select_previous_slot(&mut self);
    fn load_slot(&mut self, slot_id: u64) -> bool;
    fn save_slot(&mut self, slot_id: u64);
    fn delete_slot(&mut self, slot_id: u64);
    fn select_slot(&mut self, slot_id: u64);
    fn create_slot(&mut self);
    fn reset(&mut self);
}

/// Session-level operations: teardown and settings application.
pub trait SessionControl {
    /// Unload the current ROM, flushing mapper saves. Closing the
    /// ROM rides this trait like every other session operation so no
    /// frontend bypasses the session behind its back.
    fn unload(&mut self);
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError>;
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError>;
}

/// Frontend-facing session operations.
///
/// Hides `SessionCommand`, `SessionCommandOutcome`, and `SettingsApplyPlan`
/// from frontend code. Each frontend's state struct implements the
/// three control traits; the blanket impl below assembles this port.
/// (The former `run_command` escape hatch is gone: menu dispatch goes
/// through the typed methods, and frontends drive `SessionHandle`
/// directly for anything else.)
pub trait FrontendSession: PauseControl + SlotControl + SessionControl {}

impl<T> FrontendSession for T where T: PauseControl + SlotControl + SessionControl {}

/// Dispatch one schema session action to any frontend session.
/// Window-chrome actions never arrive here (dedicated variants).
pub fn dispatch_session_action(target: &mut dyn FrontendSession, action: ShortcutAction) {
    match action {
        ShortcutAction::TogglePause => target.toggle_pause(),
        ShortcutAction::SaveActiveSlot => {
            let _ = target.save_active_slot();
        }
        ShortcutAction::SelectNextSlot => target.select_next_slot(),
        ShortcutAction::SelectPreviousSlot => target.select_previous_slot(),
        ShortcutAction::LoadActiveSlot => {
            let _ = target.load_active_slot();
        }
        ShortcutAction::Reset => target.reset(),
        // Unreachable by construction (see above): kept exhaustive so
        // a new action fails to compile until it is routed here.
        ShortcutAction::ToggleFullscreen | ShortcutAction::OpenDebugger => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records dispatched calls. Every `FrontendSession` method lands
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

    impl SlotControl for Recorder {
        fn save_active_slot(&mut self) -> bool {
            self.calls.push("save_active_slot");
            true
        }
        fn load_active_slot(&mut self) -> bool {
            self.calls.push("load_active_slot");
            true
        }
        fn select_next_slot(&mut self) {
            self.calls.push("select_next_slot");
        }
        fn select_previous_slot(&mut self) {
            self.calls.push("select_previous_slot");
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
        fn select_slot(&mut self, _slot_id: u64) {
            self.calls.push("select_slot");
        }
        fn create_slot(&mut self) {
            self.calls.push("create_slot");
        }
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
            dispatch_session_action(&mut recorder, action);
            assert_eq!(recorder.calls.last(), Some(&name), "{action:?}");
        }
        assert_eq!(recorder.calls.len(), 6, "no action double-dispatches");
        // Chrome actions are not session operations: silence, no call.
        dispatch_session_action(&mut recorder, ShortcutAction::ToggleFullscreen);
        dispatch_session_action(&mut recorder, ShortcutAction::OpenDebugger);
        assert_eq!(recorder.calls.len(), 6);
    }
}
