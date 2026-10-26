//! Shared menu/keyboard command dispatch: one policy for every frontend.
//!
//! Frontends translate native events into [`AppCommand`] or reuse the
//! [`KeyboardShortcut`] their key routing already yields, then implement
//! [`DispatchHost`] with their toolkit. The settings-open gate, the
//! command arms, and the post-dispatch chrome sync live here, so both
//! frontends block and sync identically.

use super::KeyboardShortcut;
use super::access::{PauseControl, ResetControl, SlotIo, SlotNav};
use super::commands::SessionCommand;

/// Toolkit-neutral menu command. Frontends map native menu selections
/// (muda items, gio actions) to these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    OpenRom,
    CloseRom,
    OpenSettings,
    OpenDebugger,
    OpenHistory,
    Session(SessionCommand),
    ToggleFullscreen,
    Quit,
}

/// What the runtime must do after a dispatch. Frontends without a
/// multi-window loop may ignore every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    None,
    RomLoaded,
    Exit,
}

/// Toolkit side of shared dispatch. Session mutations go through the
/// narrow control traits (already gated per frontend); window openings
/// and chrome sync stay toolkit-specific.
pub trait DispatchHost: PauseControl + SlotIo + SlotNav + ResetControl {
    fn settings_open(&self) -> bool;
    fn open_settings(&mut self);
    fn open_debugger(&mut self);
    fn open_history(&mut self);
    fn open_rom_dialog(&mut self) -> bool;
    fn close_rom(&mut self);
    fn toggle_fullscreen(&mut self);
    fn sync_chrome(&mut self);
}

/// Dispatch one menu command under the unified contract: while settings
/// are open only Quit is processed; every other command is swallowed.
pub fn dispatch_app_command(host: &mut dyn DispatchHost, command: AppCommand) -> DispatchOutcome {
    if host.settings_open() {
        return match command {
            AppCommand::Quit => DispatchOutcome::Exit,
            _ => DispatchOutcome::None,
        };
    }
    match command {
        AppCommand::OpenRom => {
            if host.open_rom_dialog() {
                DispatchOutcome::RomLoaded
            } else {
                DispatchOutcome::None
            }
        }
        AppCommand::CloseRom => {
            host.close_rom();
            DispatchOutcome::None
        }
        AppCommand::OpenSettings => {
            host.open_settings();
            DispatchOutcome::None
        }
        AppCommand::OpenDebugger => {
            host.open_debugger();
            DispatchOutcome::None
        }
        AppCommand::OpenHistory => {
            host.open_history();
            DispatchOutcome::None
        }
        AppCommand::Session(command) => {
            dispatch_session_command(host, command);
            DispatchOutcome::None
        }
        AppCommand::ToggleFullscreen => {
            host.toggle_fullscreen();
            DispatchOutcome::None
        }
        AppCommand::Quit => DispatchOutcome::Exit,
    }
}

/// Dispatch one keyboard shortcut under the same contract.
pub fn dispatch_keyboard_shortcut(host: &mut dyn DispatchHost, shortcut: KeyboardShortcut) {
    if host.settings_open() {
        return;
    }
    match shortcut {
        KeyboardShortcut::OpenDebugger => host.open_debugger(),
        KeyboardShortcut::ToggleFullscreen => host.toggle_fullscreen(),
        KeyboardShortcut::Session(action) => dispatch_shortcut_action(host, action),
    }
    host.sync_chrome();
}

fn dispatch_session_command(host: &mut dyn DispatchHost, command: SessionCommand) {
    match command {
        SessionCommand::Pause => host.pause(),
        SessionCommand::Resume => host.resume(),
        SessionCommand::TogglePause => host.toggle_pause(),
        SessionCommand::Reset => host.reset(),
        SessionCommand::CreateSlot => host.create_slot(),
        SessionCommand::SaveActiveSlotOrNew => {
            host.save_active_slot();
        }
        SessionCommand::LoadActiveSlot => {
            host.load_active_slot();
        }
        SessionCommand::SelectActiveSlot(slot) => host.select_slot(slot),
        SessionCommand::SaveSlot(slot) => host.save_slot(slot),
        SessionCommand::LoadSlot(slot) => {
            host.load_slot(slot);
        }
        SessionCommand::DeleteSlot(slot) => host.delete_slot(slot),
        SessionCommand::SelectNextSlot => host.select_next_slot(),
        SessionCommand::SelectPreviousSlot => host.select_previous_slot(),
    }
    host.sync_chrome();
}

fn dispatch_shortcut_action(
    host: &mut dyn DispatchHost,
    action: nerust_settings_core::model::input::ShortcutAction,
) {
    use nerust_settings_core::model::input::ShortcutAction as A;
    match action {
        A::TogglePause => host.toggle_pause(),
        A::SaveActiveSlot => {
            host.save_active_slot();
        }
        A::LoadActiveSlot => {
            host.load_active_slot();
        }
        A::SelectNextSlot => host.select_next_slot(),
        A::SelectPreviousSlot => host.select_previous_slot(),
        A::Reset => host.reset(),
        // Fullscreen and debugger open ride the host's own paths so the
        // chrome sync below still runs for them.
        A::ToggleFullscreen => host.toggle_fullscreen(),
        A::OpenDebugger => host.open_debugger(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder {
        settings_open: bool,
        log: Vec<&'static str>,
    }

    impl PauseControl for Recorder {
        fn pause(&mut self) {
            self.log.push("pause");
        }
        fn resume(&mut self) {
            self.log.push("resume");
        }
        fn toggle_pause(&mut self) {
            self.log.push("toggle");
        }
    }

    impl SlotIo for Recorder {
        fn save_active_slot(&mut self) -> bool {
            self.log.push("save_active");
            true
        }
        fn load_active_slot(&mut self) -> bool {
            self.log.push("load_active");
            true
        }
        fn load_slot(&mut self, _slot: u64) -> bool {
            self.log.push("load_slot");
            true
        }
        fn save_slot(&mut self, _slot: u64) {
            self.log.push("save_slot");
        }
        fn delete_slot(&mut self, _slot: u64) {
            self.log.push("delete_slot");
        }
    }

    impl SlotNav for Recorder {
        fn select_next_slot(&mut self) {
            self.log.push("next");
        }
        fn select_previous_slot(&mut self) {
            self.log.push("prev");
        }
        fn select_slot(&mut self, _slot: u64) {
            self.log.push("select");
        }
        fn create_slot(&mut self) {
            self.log.push("create");
        }
    }

    impl ResetControl for Recorder {
        fn reset(&mut self) {
            self.log.push("reset");
        }
    }

    impl DispatchHost for Recorder {
        fn settings_open(&self) -> bool {
            self.settings_open
        }
        fn open_settings(&mut self) {
            self.log.push("settings");
        }
        fn open_debugger(&mut self) {
            self.log.push("debugger");
        }
        fn open_history(&mut self) {
            self.log.push("history");
        }
        fn open_rom_dialog(&mut self) -> bool {
            self.log.push("rom_dialog");
            true
        }
        fn close_rom(&mut self) {
            self.log.push("close");
        }
        fn toggle_fullscreen(&mut self) {
            self.log.push("fullscreen");
        }
        fn sync_chrome(&mut self) {
            self.log.push("chrome");
        }
    }

    #[test]
    fn settings_open_swallows_everything_but_quit() {
        let mut host = Recorder {
            settings_open: true,
            log: Vec::new(),
        };
        assert_eq!(
            dispatch_app_command(&mut host, AppCommand::OpenDebugger),
            DispatchOutcome::None
        );
        assert_eq!(
            dispatch_app_command(&mut host, AppCommand::Session(SessionCommand::Reset)),
            DispatchOutcome::None
        );
        assert!(host.log.is_empty());
        assert_eq!(
            dispatch_app_command(&mut host, AppCommand::Quit),
            DispatchOutcome::Exit
        );
    }

    #[test]
    fn session_command_runs_then_syncs_chrome() {
        let mut host = Recorder {
            settings_open: false,
            log: Vec::new(),
        };
        assert_eq!(
            dispatch_app_command(&mut host, AppCommand::Session(SessionCommand::Pause)),
            DispatchOutcome::None
        );
        assert_eq!(host.log, vec!["pause", "chrome"]);
    }

    #[test]
    fn rom_dialog_success_reports_loaded() {
        let mut host = Recorder {
            settings_open: false,
            log: Vec::new(),
        };
        assert_eq!(
            dispatch_app_command(&mut host, AppCommand::OpenRom),
            DispatchOutcome::RomLoaded
        );
    }
}
