//! Menu/keyboard dispatch, ROM lifecycle, and the session delegate.

use nerust_gui_shell::registry::SystemRegistry;

use super::HostState;
use super::{HostAction, element_state_to_pressed};
use tao::event::KeyEvent;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionError,
    access::SettingsResult,
    access::{PauseControl, SessionControl, SlotControl},
    commands::SessionCommand,
};

use crate::event::MenuCommand;
use crate::ports::WindowOpener;

impl HostState {
    pub(crate) fn on_menu_command(
        &mut self,
        command: MenuCommand,
        opener: &dyn WindowOpener,
    ) -> HostAction {
        if self.settings_open() {
            return match command {
                MenuCommand::Quit => {
                    self.prepare_close();
                    HostAction::Exit
                }
                _ => HostAction::None,
            };
        }
        match command {
            MenuCommand::Open => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            MenuCommand::Close => {
                self.close_rom();
                HostAction::None
            }
            MenuCommand::Settings => {
                self.open_settings_window(opener);
                HostAction::None
            }
            MenuCommand::Debugger => {
                self.open_debug_windows(opener, false);
                HostAction::None
            }
            MenuCommand::NotificationHistory => {
                self.open_history(opener);
                HostAction::None
            }
            MenuCommand::Session(command) => {
                let _ = self.session.run_command(command);
                self.sync_menu_state();
                self.refresh_window_title();
                HostAction::None
            }
            MenuCommand::Quit => {
                self.prepare_close();
                HostAction::Exit
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent) {
        if self.settings_open() {
            return;
        }
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = crate::convert::keycode_to_key(input.physical_key)
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_input();
    }

    fn open_rom_dialog(&mut self) -> bool {
        let language = self.session.settings_snapshot().shared.general.language;
        let filters = Self::rom_dialog_filters(&self.ctx.registry);
        let Some(path) = self.dialogs.pick_rom_file(language, filters) else {
            return false;
        };
        if let Err(detail) = self.load_path(&path) {
            self.dialogs.show_load_error(language, &detail);
            return false;
        }
        true
    }

    /// ROM-picker filters from the registered cores, extracted pure
    /// so the dialog wiring stays unit-testable. Cores without
    /// extensions never reach the picker.
    fn rom_dialog_filters(registry: &SystemRegistry) -> Vec<(String, Vec<String>)> {
        registry
            .all()
            .iter()
            .filter_map(|factory| {
                let extensions = factory.supported_extensions();
                (!extensions.is_empty()).then(|| {
                    (
                        format!("{} ROM", factory.display_name()),
                        extensions.iter().map(|item| item.to_string()).collect(),
                    )
                })
            })
            .collect()
    }

    fn close_rom(&mut self) {
        SessionControl::unload(self);
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        use nerust_gui_shell::session::access::dispatch_session_action;
        match shortcut {
            // The debugger open rides the same schema as every other
            // shortcut (default F4, rebindable in Settings): it only
            // sets the flag here because opening needs the event loop,
            // which the runtime arm owns.
            KeyboardShortcut::OpenDebugger => {
                self.debug.request_debug_open();
                return;
            }
            KeyboardShortcut::Session(action) => dispatch_session_action(self, action),
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }
}

impl PauseControl for HostState {
    fn pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::Pause);
    }
    fn resume(&mut self) {
        let _ = self.session.run_command(SessionCommand::Resume);
    }
    fn toggle_pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::TogglePause);
    }
}

impl SlotControl for HostState {
    fn save_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.session
            .run_command(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }
    fn save_slot(&mut self, slot_id: u64) {
        let _ = self.session.run_command(SessionCommand::SaveSlot(slot_id));
    }
    fn delete_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::DeleteSlot(slot_id));
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
    fn reset(&mut self) {
        let _ = self.session.run_command(SessionCommand::Reset);
    }
}

impl SessionControl for HostState {
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        Ok(SettingsResult {
            // The renderer follows presentation, not the session: a
            // live filter swap rebuilds no core but still needs the new
            // GPU assets (same rule as the android twin).
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::HostState;
    use crate::event::MenuCommand;
    use crate::runtime::host::HostAction;
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::test_session::FakeRomLoader;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_gui_shell::session::commands::SessionCommand;
    use nerust_gui_shell::session::osd::ModalSource;

    #[test]
    fn rom_filters_empty_without_cores() {
        let registry = SystemRegistry::new(Vec::new());
        assert!(HostState::rom_dialog_filters(&registry).is_empty());
    }

    #[test]
    fn rom_dialog_cancel_picks_nothing() {
        let support = TestHost::with_pick(None);
        let mut host = support.host;
        assert!(!host.open_rom_dialog());
        assert!(support.loader.attempted().is_empty());
        assert!(support.dialogs.errors().is_empty());
    }

    #[test]
    fn rom_dialog_success_loads_picked_path() {
        let path = std::path::PathBuf::from("/tmp/game.nes");
        let support = TestHost::with_pick(Some(path.clone()));
        let mut host = support.host;
        assert!(host.open_rom_dialog());
        assert_eq!(support.loader.attempted(), [path]);
        assert!(support.dialogs.errors().is_empty());
    }

    #[test]
    fn rom_dialog_failure_surfaces_error() {
        let path = std::path::PathBuf::from("/tmp/broken.nes");
        let loader = FakeRomLoader::failing("bad rom");
        let support = TestHost::with_loader_and_pick(loader, Some(path.clone()));
        let mut host = support.host;
        assert!(!host.open_rom_dialog());
        assert_eq!(support.loader.attempted(), [path]);
        assert_eq!(support.dialogs.errors(), ["ROM load failed: bad rom"]);
    }

    #[test]
    fn settings_open_guard_swallows_non_quit() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Open, &opener),
            HostAction::None
        );
        assert!(opener.calls().is_empty());
        assert!(support.loader.attempted().is_empty());
    }

    #[test]
    fn quit_exits_and_closes() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Quit, &opener),
            HostAction::Exit
        );
    }

    #[test]
    fn open_menu_routes_dialog_result() {
        let path = std::path::PathBuf::from("/tmp/game.nes");
        let support = TestHost::with_pick(Some(path.clone()));
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Open, &opener),
            HostAction::RomLoaded
        );
        assert_eq!(support.loader.attempted(), [path]);
    }

    #[test]
    fn close_menu_unloads_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Close, &opener),
            HostAction::None
        );
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn session_menu_runs_command_and_syncs() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Session(SessionCommand::TogglePause), &opener,),
            HostAction::None
        );
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn settings_menu_records_open_attempt() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Settings, &opener),
            HostAction::None
        );
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!host.settings_open());
    }

    #[test]
    fn debugger_menu_records_open_attempt() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Debugger, &opener),
            HostAction::None
        );
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!host.debug.is_debug_open());
    }

    #[test]
    fn session_menu_runs_every_command_without_core() {
        use nerust_gui_shell::session::commands::SessionCommand as Cmd;
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        let before = support.menu.updates().len();
        for command in [
            Cmd::Pause,
            Cmd::Resume,
            Cmd::TogglePause,
            Cmd::Reset,
            Cmd::CreateSlot,
            Cmd::SaveActiveSlotOrNew,
            Cmd::LoadActiveSlot,
            Cmd::SelectActiveSlot(7),
            Cmd::SaveSlot(7),
            Cmd::LoadSlot(7),
            Cmd::DeleteSlot(7),
            Cmd::SelectNextSlot,
            Cmd::SelectPreviousSlot,
        ] {
            assert_eq!(
                host.on_menu_command(MenuCommand::Session(command), &opener),
                HostAction::None
            );
        }
        // Every command syncs the menu, none opens a window.
        assert_eq!(support.menu.updates().len(), before + 13);
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn session_delegates_stay_total_without_core() {
        use nerust_gui_shell::session::access::{PauseControl, SessionControl, SlotControl};
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.session.paused());
        host.pause();
        host.resume();
        host.toggle_pause();
        assert!(!host.session.paused());
        assert!(!host.save_active_slot());
        assert!(!host.load_active_slot());
        host.select_next_slot();
        host.select_previous_slot();
        assert!(!host.load_slot(99));
        host.save_slot(99);
        host.delete_slot(99);
        host.select_slot(99);
        host.create_slot();
        host.reset();
        host.unload();
        assert!(!host.session.loaded());
    }

    #[test]
    fn error_modal_returns_without_blocking() {
        use nerust_gui_shell::session::commands::SessionCommand;
        let support = TestHost::new();
        let mut host = support.host;
        // No core: TogglePause fails into a CommandFailed error,
        // which routes to the modal path (error_modal defaults on).
        let _ = host.session.run_command(SessionCommand::TogglePause);
        host.poll_notifications();
        // Bounded wait for the background ack. The loop itself never
        // blocked: rfd runs on its own thread (see `ModalFlight`).
        // FakeDialogs answers instantly, so this exits on an early pass.
        let mut tries = 0;
        while support.dialogs.errors().is_empty() || host.osd.manager.modal_pending() != 0 {
            host.poll_notifications();
            tries += 1;
            assert!(tries < 500, "modal ack never arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("TogglePause"));
        assert_eq!(host.osd.manager.modal_pending(), 0);
    }

    #[test]
    fn dead_dialog_thread_unwedges_the_queue() {
        use super::super::osd::ModalFlight;
        let support = TestHost::new();
        let mut host = support.host;
        // Simulate a dialog thread that died before acking: the
        // sender is dropped without sending (Disconnected).
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        host.osd.modal_flight = Some(ModalFlight {
            done: rx,
            resume_after: false,
        });
        host.poll_notifications();
        // Reaped as a failed ack: flight gone, queue unblocked.
        assert!(host.osd.modal_flight.is_none());
        assert_eq!(host.osd.manager.modal_pending(), 0);
    }

    #[test]
    fn idle_poll_touches_nothing() {
        let support = TestHost::new();
        let mut host = support.host;
        // No notifications, no windows: drain, expiry, dirty gate,
        // and the history fast path all no-op without a core.
        host.poll_notifications();
        host.poll_notifications();
        assert!(support.dialogs.errors().is_empty());
        assert_eq!(host.osd.manager.modal_pending(), 0);
        assert!(host.osd.history_sync_key.is_none());
    }

    #[test]
    fn keyboard_shortcuts_route_without_keys() {
        use nerust_gui_shell::session::KeyboardShortcut;
        use nerust_settings_core::model::input::ShortcutAction;
        let support = TestHost::new();
        let mut host = support.host;
        host.apply_keyboard_shortcut(KeyboardShortcut::OpenDebugger);
        assert!(host.take_debug_open_request());
        assert!(support.menu.updates().is_empty());
        host.apply_keyboard_shortcut(KeyboardShortcut::Session(ShortcutAction::TogglePause));
        host.apply_keyboard_shortcut(KeyboardShortcut::ToggleFullscreen);
        assert_eq!(support.menu.updates().len(), 2);
    }

    #[test]
    fn session_verbs_stay_total_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        host.resume_session();
        host.pause_session();
        host.clear_keys();
        host.sync_fullscreen_from_settings();
        let _ = host.persist_fullscreen_default(false);
        assert!(!host.session.loaded());
    }

    #[test]
    fn apply_paths_stay_total_without_core() {
        use nerust_gui_shell::session::access::SessionControl;
        let support = TestHost::new();
        let mut host = support.host;
        let snapshot = host.session.settings_snapshot().clone();
        let _ = host.apply_settings(snapshot);
        let _ = host.set_fullscreen_default(true);
        assert!(!host.session.loaded());
    }
}
