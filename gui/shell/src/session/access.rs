use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::model::input::ShortcutAction;

use crate::session::SessionError;

/// Summary of a settings-apply operation, visible to frontends.
/// Only the flags that frontends actually act on.
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

/// Blanket narrow ports over the legacy fat trait; new code depends on narrow traits.
impl<T: SlotControl + ?Sized> SlotIo for T {
    fn load_slot(&mut self, slot_id: u64) -> bool {
        SlotControl::load_slot(self, slot_id)
    }
    fn save_slot(&mut self, slot_id: u64) {
        SlotControl::save_slot(self, slot_id)
    }
    fn delete_slot(&mut self, slot_id: u64) {
        SlotControl::delete_slot(self, slot_id)
    }
    fn save_active_slot(&mut self) -> bool {
        SlotControl::save_active_slot(self)
    }
    fn load_active_slot(&mut self) -> bool {
        SlotControl::load_active_slot(self)
    }
}

impl<T: SlotControl + ?Sized> SlotNav for T {
    fn select_slot(&mut self, slot_id: u64) {
        SlotControl::select_slot(self, slot_id)
    }
    fn select_next_slot(&mut self) {
        SlotControl::select_next_slot(self)
    }
    fn select_previous_slot(&mut self) {
        SlotControl::select_previous_slot(self)
    }
    fn create_slot(&mut self) {
        SlotControl::create_slot(self)
    }
}

impl<T: SlotControl + ?Sized> ResetControl for T {
    fn reset(&mut self) {
        SlotControl::reset(self)
    }
}

/// Legacy fat shim for out-of-crate frontends; new code uses narrow traits.
/// Kept for compatibility: blanket impls forward narrow calls here.
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

/// Dispatch one schema session action; chrome actions use dedicated variants.
pub fn dispatch_session_action<T>(target: &mut T, action: ShortcutAction)
where
    T: PauseControl + SlotIo + SlotNav + ResetControl + ?Sized,
{
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
        ShortcutAction::ToggleFullscreen | ShortcutAction::OpenDebugger => {}
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
        // Narrow ports reach the recorder through the blanket impls:
        // no adapter stands between the fat trait and its halves.
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
