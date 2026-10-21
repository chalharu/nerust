//! Menu/keyboard dispatch, ROM lifecycle, and the session delegate.

use nerust_gui_shell::registry::SystemRegistry;

use super::HostState;
use super::{HostAction, element_state_to_pressed};
use tao::event::KeyEvent;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionError,
    access::SettingsResult,
    access::{PauseControl, ResetControl, SessionControl, SlotIo, SlotNav},
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
                match command {
                    SessionCommand::Pause => self.pause(),
                    SessionCommand::Resume => self.resume(),
                    SessionCommand::TogglePause => self.toggle_pause(),
                    command => {
                        let _ = self.session.run_command(command);
                    }
                }
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, false);
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

    /// ROM-picker filters from the registered cores; cores without extensions never reach.
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
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        use nerust_gui_shell::session::access::dispatch_session_action;
        match shortcut {
            // Only sets the flag here; opening needs the event loop the runtime arm owns.
            KeyboardShortcut::OpenDebugger => {
                self.request_debug_open();
                return;
            }
            KeyboardShortcut::Session(action) => {
                let _ = dispatch_session_action(self, action);
            }
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, false);
    }
}

impl PauseControl for HostState {
    fn pause(&mut self) {
        self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::User, true);
    }
    fn resume(&mut self) {
        self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::User, false);
    }
    fn toggle_pause(&mut self) {
        self.set_pause_owner(
            nerust_frontend_kernel::pause::PauseOwner::User,
            !self.session.tick_metrics().paused,
        );
    }
}

impl SlotIo for HostState {
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
}

impl SlotNav for HostState {
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
}

impl ResetControl for HostState {
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
        let plan = self.session.apply_settings(settings);
        self.reconcile_pause();
        let plan = plan?;
        Ok(SettingsResult {
            // The renderer follows presentation, not the session.
            renderer_needs_rebuild: plan.renderer_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required || plan.window_settings_changed,
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
    use crate::testkit::StubAux;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_gui_shell::session::commands::SessionCommand;

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
        assert_eq!(
            support.dialogs.errors(),
            ["ROM load failed: no emulation core active"]
        );
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

    fn settings_blocked_host() -> TestHost {
        let support = TestHost::new();
        let mut host = support.host;
        host.aux
            .insert(Box::new(StubAux::new(crate::aux::focus::AuxKind::Settings)));
        assert!(host.settings_open());
        TestHost {
            host,
            menu: support.menu,
            loader: support.loader,
            dialogs: support.dialogs,
        }
    }

    #[test]
    fn open_settings_blocks_menu_commands() {
        use crate::event::MenuCommand as Cmd;
        use nerust_gui_shell::session::commands::SessionCommand;
        let support = settings_blocked_host();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        for command in [
            Cmd::Open,
            Cmd::Close,
            Cmd::Settings,
            Cmd::Debugger,
            Cmd::NotificationHistory,
            Cmd::Session(SessionCommand::TogglePause),
        ] {
            assert_eq!(host.on_menu_command(command, &opener), HostAction::None);
        }
        assert!(opener.calls().is_empty());
        assert!(support.loader.attempted().is_empty());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn open_settings_lets_quit_through() {
        let support = settings_blocked_host();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::Quit, &opener),
            HostAction::Exit
        );
        assert!(host.close_exited());
    }

    #[test]
    fn history_menu_records_open_attempt() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_menu_command(MenuCommand::NotificationHistory, &opener),
            HostAction::None
        );
        assert_eq!(opener.calls(), [OpenerCall::History]);
        assert!(!host.aux.contains(crate::aux::focus::AuxKind::History));
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
        assert!(!host.is_debug_open());
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
        // dirty gate: identical chrome skips the IPC. Only the
        // first sync and the active-slot selection change the sig.
        assert_eq!(support.menu.updates().len(), before + 2);
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn session_delegates_stay_total_without_core() {
        use nerust_gui_shell::session::access::{
            PauseControl, ResetControl, SessionControl, SlotIo, SlotNav,
        };
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.session.tick_metrics().paused);
        host.pause();
        host.resume();
        host.toggle_pause();
        assert!(!host.session.tick_metrics().paused);
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
        assert!(!host.session.tick_metrics().loaded);
    }

    #[test]
    fn error_modal_returns_without_blocking() {
        let support = TestHost::new();
        let mut host = support.host;
        host.session.report_external_error(
            nerust_gui_shell::session::notification::FailureOrigin::Host,
            "test error".into(),
        );
        let ctx = host.frame_ctx();
        host.poll_notifications(&ctx);
        // Bounded wait for the background ack; FakeDialogs answers instantly.
        let mut tries = 0;
        while support.dialogs.errors().is_empty() || host.osd.modal_pending() != 0 {
            let ctx = host.frame_ctx();
            host.poll_notifications(&ctx);
            tries += 1;
            assert!(tries < 500, "modal ack never arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("test error"));
        assert_eq!(host.osd.modal_pending(), 0);
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
        host.osd
            .inject_modal_flight_for_test(ModalFlight { done: rx });
        let ctx = host.frame_ctx();
        host.poll_notifications(&ctx);
        // Reaped as a failed ack: flight gone, queue unblocked.
        assert!(!host.osd.modal_showing());
        assert_eq!(host.osd.modal_pending(), 0);
    }

    #[test]
    fn idle_poll_touches_nothing() {
        let support = TestHost::new();
        let mut host = support.host;
        // No notifications, no windows: drain, expiry, dirty gate,
        // and the history fast path all no-op without a core.
        let ctx = host.frame_ctx();
        host.poll_notifications(&ctx);
        let ctx = host.frame_ctx();
        host.poll_notifications(&ctx);
        assert!(support.dialogs.errors().is_empty());
        assert_eq!(host.osd.modal_pending(), 0);
        assert!(host.osd.history_key().is_none());
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
        // dirty gate: the pause sync pushes once (headless toggle
        // without a window adds no second change).
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn session_verbs_stay_total_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        host.resume_session();
        super::PauseControl::pause(&mut host);
        host.clear_keys();
        host.sync_fullscreen_from_settings();
        let _ = host.persist_fullscreen_default(false);
        assert!(!host.session.tick_metrics().loaded);
    }

    #[test]
    fn apply_paths_stay_total_without_core() {
        use nerust_gui_shell::session::access::SessionControl;
        let support = TestHost::new();
        let mut host = support.host;
        let snapshot = host.session.settings_snapshot().clone();
        let _ = host.apply_settings(snapshot);
        let _ = host.set_fullscreen_default(true);
        assert!(!host.session.tick_metrics().loaded);
    }
}
