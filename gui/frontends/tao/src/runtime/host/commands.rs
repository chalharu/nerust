//! Menu/keyboard dispatch, ROM lifecycle, and the session delegate.

use super::HostState;
use super::{HostAction, element_state_to_pressed};
use tao::event::KeyEvent;

use nerust_frontend_host::{
    event::{CommandEffect, EventHandler, EventOutcome},
    focus::{AuxKind, FocusEvent},
    session::SessionEvent,
};

#[cfg(test)]
use crate::event::MenuCommand;
use crate::event::UserEvent;
use crate::ports::WindowOpener;

impl HostState {
    #[cfg(test)]
    pub(crate) fn on_menu_command(
        &mut self,
        command: MenuCommand,
        opener: &dyn WindowOpener,
    ) -> HostAction {
        self.on_event(UserEvent::Menu(command), opener)
    }

    pub(crate) fn on_event(&mut self, event: UserEvent, opener: &dyn WindowOpener) -> HostAction {
        self.handle_native_event(event, Some(opener))
    }

    fn dispatch_event(&mut self, event: UserEvent) -> Option<EventOutcome> {
        let settings_open = self.settings_open();
        let handler: &mut dyn EventHandler = &mut self.core;
        match handler.handle_event(event, settings_open) {
            Ok(outcome) => Some(outcome),
            Err(_) => {
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, false);
                None
            }
        }
    }

    pub(crate) fn on_focus_event(&mut self, event: FocusEvent) {
        let _ = self.dispatch_event(UserEvent::Focus(event));
    }

    fn handle_native_event(
        &mut self,
        event: UserEvent,
        opener: Option<&dyn WindowOpener>,
    ) -> HostAction {
        let outcome = match self.dispatch_event(event) {
            Some(outcome) if !outcome.handled => return HostAction::None,
            Some(outcome) => outcome,
            None => return HostAction::None,
        };
        match outcome.effect {
            Some(CommandEffect::OpenRom) => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            Some(CommandEffect::OpenAux(kind)) => {
                match (kind, opener) {
                    (AuxKind::Settings, Some(opener)) => self.open_settings_window(opener),
                    (AuxKind::Debug, Some(opener)) => self.open_debug_windows(opener, false),
                    (AuxKind::History, Some(opener)) => self.open_history(opener),
                    (AuxKind::Debug, None) => self.request_debug_open(),
                    _ => (),
                }
                HostAction::None
            }
            Some(CommandEffect::Quit) => HostAction::Exit,
            Some(CommandEffect::LiveTick) => {
                self.live_tick();
                HostAction::None
            }
            Some(CommandEffect::About) => {
                // Standard native application-menu resources present About.
                HostAction::None
            }
            effect => {
                if effect == Some(CommandEffect::ToggleFullscreen) {
                    self.toggle_fullscreen();
                }
                if matches!(event, UserEvent::Focus(_))
                    || (effect.is_none()
                        && outcome.session.command.is_none()
                        && !outcome.session.settings_changed
                        && matches!(event, UserEvent::Session(_)))
                {
                    return HostAction::None;
                }
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, true);
                HostAction::None
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent, opener: &dyn WindowOpener) {
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = crate::convert::keycode_to_key(input.physical_key)
        {
            self.on_event(
                UserEvent::Session(SessionEvent::Keyboard { key, pressed }),
                opener,
            );
        }
    }

    fn open_rom_dialog(&mut self) -> bool {
        let titles = self.core.dialog_titles();
        let filters = self.core.rom_dialog_filters();
        let Some(path) = self.dialogs.pick_rom_file(titles.open, filters) else {
            return false;
        };
        if let Err(detail) = self.load_path(&path) {
            self.dialogs.show_load_error(titles.error, &detail);
            return false;
        }
        true
    }

    #[cfg(test)]
    fn apply_keyboard_shortcut(&mut self, shortcut: nerust_gui_shell::session::KeyboardShortcut) {
        self.handle_native_event(UserEvent::Session(SessionEvent::Shortcut(shortcut)), None);
    }
}

#[cfg(test)]
mod tests {
    use crate::event::MenuCommand;
    use crate::runtime::host::HostAction;
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::test_session::FakeRomLoader;
    use crate::testkit::StubAux;
    use nerust_frontend_host::commands::NativeRole;
    use nerust_frontend_host::commands::SessionCommand;

    #[test]
    fn rom_filters_empty_without_cores() {
        let support = TestHost::new();
        assert!(support.host.rom_dialog_filters().is_empty());
    }

    #[test]
    fn rom_filters_lead_with_unified_all_roms() {
        let support = TestHost::with_nes();
        let filters = support.host.rom_dialog_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].0, "All ROMs");
        assert_eq!(filters[0].1, ["nes"]);
        assert_eq!(filters[1].0, "NES ROM");
        assert_eq!(filters[1].1, ["nes"]);
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
        use nerust_frontend_host::commands::SessionCommand;
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
            host.on_menu_command(MenuCommand::Native(NativeRole::Quit), &opener),
            HostAction::Exit
        );
        // Intent only: host teardown waits for the runtime's ordered close.
        assert!(!host.close_exited());
    }

    #[test]
    fn canonical_events_share_external_settings_guard_and_effects() {
        use crate::event::UserEvent;
        use nerust_frontend_host::{focus::FocusEvent, session::SessionEvent};
        use nerust_gui_shell::session::KeyboardShortcut;
        let support = settings_blocked_host();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        // Only the native registry is open: the external flag, not a Host
        // pause owner, must guard both commands and keyboard shortcuts.
        assert!(!host.owns_pause(wakui::pause::PauseOwner::Settings));
        for event in [
            UserEvent::Menu(MenuCommand::Native(NativeRole::About)),
            UserEvent::Session(SessionEvent::Command(SessionCommand::SelectActiveSlot(42))),
            UserEvent::Session(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger)),
            UserEvent::Session(SessionEvent::Keyboard {
                key: wakui::Key::F4,
                pressed: true,
            }),
        ] {
            assert_eq!(host.on_event(event, &opener), HostAction::None);
        }
        assert!(host.session().active_slot_id().is_none());
        assert!(support.menu.updates().is_empty());
        assert!(opener.calls().is_empty());
        assert_eq!(
            host.on_event(UserEvent::Focus(FocusEvent::Main(false)), &opener),
            HostAction::None
        );
        assert!(!host.main_active());
        assert_eq!(
            host.on_event(UserEvent::LiveTick, &opener),
            HostAction::None
        );
        assert_eq!(
            host.on_event(
                UserEvent::Menu(MenuCommand::Native(NativeRole::Quit)),
                &opener
            ),
            HostAction::Exit
        );
        assert!(!host.close_exited());
    }

    #[test]
    fn canonical_shortcut_effect_opens_native_debugger() {
        use crate::event::UserEvent;
        use nerust_frontend_host::session::SessionEvent;
        use nerust_gui_shell::session::KeyboardShortcut;
        let mut host = TestHost::new().host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            host.on_event(
                UserEvent::Session(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger)),
                &opener
            ),
            HostAction::None
        );
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
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
            host.on_menu_command(MenuCommand::Native(NativeRole::Quit), &opener),
            HostAction::Exit
        );
        // Intent only: no teardown before the runtime's ordered close.
        assert!(!host.close_exited());
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
        use nerust_frontend_host::commands::SessionCommand as Cmd;
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
        assert!(!host.tick().paused);
        host.pause();
        host.resume();
        host.toggle_pause();
        assert!(!host.tick().paused);
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
        assert!(!host.tick().loaded);
    }

    #[test]
    fn error_modal_returns_without_blocking() {
        let support = TestHost::new();
        let mut host = support.host;
        host.report_host_error("test error".into());
        let ctx = host.frame_ctx();
        host.poll_notifications(&ctx);
        // Bounded wait for the background ack; FakeDialogs answers instantly.
        let mut tries = 0;
        while support.dialogs.errors().is_empty() || host.osd_modal_pending() {
            let ctx = host.frame_ctx();
            host.poll_notifications(&ctx);
            tries += 1;
            assert!(tries < 500, "modal ack never arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("test error"));
        assert_eq!(host.osd_pending(), 0);
        assert!(!host.osd_modal_pending());
    }

    #[test]
    fn dead_dialog_thread_unwedges_the_queue() {
        use wakui::pause::PauseOwner;
        let support = TestHost::new();
        let mut host = support.host;
        host.set_pause_owner(PauseOwner::Settings, true);
        // Simulate a dialog thread that died before acking: the
        // sender is dropped without sending (Disconnected).
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        let ctx = host.frame_ctx();
        host.report_host_error("disconnected dialog".into());
        let presentation = host
            .core
            .pump_osd(&ctx, std::time::Instant::now())
            .modal
            .unwrap();
        assert!(host.attach_osd_modal_receiver(presentation.token, rx));
        assert!(host.owns_pause(PauseOwner::Modal));
        host.poll_notifications(&ctx);
        // Reaped as a failed ack: flight gone, queue unblocked.
        assert!(!host.osd_modal_pending());
        assert_eq!(host.osd_pending(), 0);
        assert!(!host.owns_pause(PauseOwner::Modal));
        assert!(host.owns_pause(PauseOwner::Settings));
    }

    #[test]
    fn disconnected_dialog_chains_through_native_presenter_before_modal_release() {
        use wakui::pause::PauseOwner;
        let support = TestHost::new();
        let mut host = support.host;
        host.set_pause_owner(PauseOwner::Focus, true);
        host.report_host_error("dead first dialog".into());
        let ctx = host.frame_ctx();
        let first = host
            .core
            .pump_osd(&ctx, std::time::Instant::now())
            .modal
            .unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(host.attach_osd_modal_receiver(first.token, receiver));
        host.report_host_error("chained native dialog".into());
        drop(sender);

        host.poll_notifications(&ctx);
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("chained native dialog"));
        assert!(host.osd_modal_pending());
        assert!(host.osd_flight_pending());
        assert!(host.owns_pause(PauseOwner::Modal));
        assert!(host.owns_pause(PauseOwner::Focus));
        assert_eq!(host.osd_pending(), 0);

        host.poll_notifications(&ctx);
        assert!(!host.osd_modal_pending());
        assert!(!host.osd_flight_pending());
        assert!(!host.owns_pause(PauseOwner::Modal));
        assert!(host.owns_pause(PauseOwner::Focus));
        assert_eq!(support.dialogs.errors().len(), 1);
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
        assert_eq!(host.osd_pending(), 0);
        assert!(host.history_key().is_none());
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
        use nerust_gui_shell::session::access::PauseControl;
        let support = TestHost::new();
        let mut host = support.host;
        host.resume_session();
        PauseControl::pause(&mut host.core);
        host.clear_keys();
        host.sync_fullscreen_from_settings();
        let _ = host.persist_fullscreen_default(false);
        assert!(!host.tick().loaded);
    }

    #[test]
    fn apply_paths_stay_total_without_core() {
        use nerust_gui_shell::session::access::SessionControl;
        let support = TestHost::new();
        let mut host = support.host;
        let snapshot = host.settings_snapshot().clone();
        let _ = host.apply_settings(snapshot);
        let _ = host.set_fullscreen_default(true);
        assert!(!host.tick().loaded);
    }
}
