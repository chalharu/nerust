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

/// Save/load/delete leg of slot control. Split from navigation so
/// test fakes and frontends depend only on the half they use.
/// Blanket-impl'd from `SlotControl` so existing implementors keep
/// compiling; new code should bound on these narrow traits.
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

/// Save-slot control: slot CRUD plus reset. Legacy fat trait: the
/// tao, gtk, and android frontends implement this out of crate, so it
/// cannot be deleted without breaking them. New code implements the
/// narrow [`SlotIo`]/[`SlotNav`]/[`ResetControl`] traits instead and
/// reaches legacy implementors through [`SlotControlAdapter`].
/// Kept as a shim: removing it breaks out-of-crate frontends.
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

/// Explicit adapter from the legacy fat [`SlotControl`] to the narrow
/// ports. One named struct instead of blanket impls, so coherence
/// never surprises a new narrow implementor: only this adapter
/// bridges the two shapes.
#[derive(Debug)]
pub struct SlotControlAdapter<S>(pub S);

/// Reference forwarding: `&mut T` speaks `SlotControl` whenever `T`
/// does, so adapters compose without consuming the recorder.
impl<T: SlotControl> SlotControl for &mut T {
    fn load_slot(&mut self, slot_id: u64) -> bool {
        T::load_slot(self, slot_id)
    }
    fn save_slot(&mut self, slot_id: u64) {
        T::save_slot(self, slot_id)
    }
    fn delete_slot(&mut self, slot_id: u64) {
        T::delete_slot(self, slot_id)
    }
    fn save_active_slot(&mut self) -> bool {
        T::save_active_slot(self)
    }
    fn load_active_slot(&mut self) -> bool {
        T::load_active_slot(self)
    }
    fn select_slot(&mut self, slot_id: u64) {
        T::select_slot(self, slot_id)
    }
    fn select_next_slot(&mut self) {
        T::select_next_slot(self)
    }
    fn select_previous_slot(&mut self) {
        T::select_previous_slot(self)
    }
    fn create_slot(&mut self) {
        T::create_slot(self)
    }
    fn reset(&mut self) {
        T::reset(self)
    }
}

impl<S: SlotControl> SlotIo for SlotControlAdapter<S> {
    fn load_slot(&mut self, slot_id: u64) -> bool {
        SlotControl::load_slot(&mut self.0, slot_id)
    }
    fn save_slot(&mut self, slot_id: u64) {
        SlotControl::save_slot(&mut self.0, slot_id)
    }
    fn delete_slot(&mut self, slot_id: u64) {
        SlotControl::delete_slot(&mut self.0, slot_id)
    }
    fn save_active_slot(&mut self) -> bool {
        SlotControl::save_active_slot(&mut self.0)
    }
    fn load_active_slot(&mut self) -> bool {
        SlotControl::load_active_slot(&mut self.0)
    }
}

impl<S: SlotControl> SlotNav for SlotControlAdapter<S> {
    fn select_slot(&mut self, slot_id: u64) {
        SlotControl::select_slot(&mut self.0, slot_id)
    }
    fn select_next_slot(&mut self) {
        SlotControl::select_next_slot(&mut self.0)
    }
    fn select_previous_slot(&mut self) {
        SlotControl::select_previous_slot(&mut self.0)
    }
    fn create_slot(&mut self) {
        SlotControl::create_slot(&mut self.0)
    }
}

impl<S: SlotControl> ResetControl for SlotControlAdapter<S> {
    fn reset(&mut self) {
        SlotControl::reset(&mut self.0)
    }
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
/// Hides command/outcome/plan types; blanket impl assembles the port.
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

    #[test]
    fn recorder_pins_each_slot_and_lifecycle_call() {
        // The remaining Recorder methods are not reachable through
        // dispatch; pin their recording through the explicit adapter,
        // which is the only bridge from the legacy fat trait to the
        // narrow ports. New code implements the narrow traits direct.
        let mut recorder = Recorder::default();
        recorder.pause();
        recorder.resume();
        {
            let mut adapted = SlotControlAdapter(&mut recorder);
            assert!(SlotIo::save_active_slot(&mut adapted));
            assert!(SlotIo::load_active_slot(&mut adapted));
            SlotIo::save_slot(&mut adapted, 1);
            SlotIo::load_slot(&mut adapted, 1);
            SlotIo::delete_slot(&mut adapted, 1);
            SlotNav::select_slot(&mut adapted, 1);
            SlotNav::create_slot(&mut adapted);
            ResetControl::reset(&mut adapted);
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
