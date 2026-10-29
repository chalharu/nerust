//! Menu/keyboard dispatch, file lifecycle, and the session delegate.

use super::HostState;
use super::{HostAction, element_state_to_pressed};
use tao::event::KeyEvent;

use wakui_core::focus::FocusEvent;
use wakui_core::{
    event::{CommandEffect, EventOutcome},
    session::SessionEvent,
};

#[cfg(test)]
use crate::event::CommandRef;
use crate::event::UserEvent;
use crate::ports::WindowOpener;

impl<K: Copy + Eq + std::hash::Hash + 'static> HostState<K> {
    #[cfg(test)]
    pub(crate) fn on_menu_command(
        &mut self,
        command: CommandRef,
        opener: &dyn WindowOpener,
    ) -> HostAction {
        self.on_event(UserEvent::Menu(command), opener)
    }

    pub(crate) fn on_event(&mut self, event: UserEvent, opener: &dyn WindowOpener) -> HostAction {
        self.handle_native_event(event, Some(opener))
    }

    fn dispatch_event(&mut self, event: UserEvent) -> Option<EventOutcome<K>> {
        // The modal guard lives view-side; flavors forward every event.
        match self.core.handle_event(event) {
            Ok(outcome) => Some(outcome),
            Err(_) => {
                self.sync_chrome(false);
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
            Some(CommandEffect::OpenFile) => {
                if self.open_file_dialog() {
                    HostAction::FileLoaded
                } else {
                    HostAction::None
                }
            }
            Some(CommandEffect::OpenAux(kind)) => {
                match opener {
                    Some(opener) => self.open_aux_window(kind, opener),
                    None => self.core.request_open(kind),
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
                        && matches!(event, UserEvent::Session(_)))
                {
                    return HostAction::None;
                }
                self.sync_chrome(true);
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

    fn open_file_dialog(&mut self) -> bool {
        let titles = self.core.dialog_titles();
        let filters = self.core.file_dialog_filters();
        let Some(path) = self.dialogs.pick_file(titles.open, filters) else {
            return false;
        };
        if let Err(detail) = self.load_path(&path) {
            self.dialogs.show_load_error(titles.error, &detail);
            return false;
        }
        true
    }

    #[cfg(test)]
    fn apply_keyboard_f4(&mut self) {
        use wakui_core::Key;
        self.handle_native_event(
            UserEvent::Session(SessionEvent::Keyboard {
                key: Key::F4,
                pressed: true,
            }),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::HostAction;
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::testkit::{StubAux, TestMenu as M, TestSession as S};
    use wakui_core::app::FrontendApp as _;

    #[test]
    fn file_filters_empty_without_cores() {
        let support = TestHost::new();
        assert!(support.host.core.file_dialog_filters().is_empty());
    }

    #[test]
    fn file_filters_list_canned_kinds() {
        let support = TestHost::with_nes();
        let filters = support.host.core.file_dialog_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].0, "Data files");
        assert_eq!(filters[0].1, ["dat"]);
    }

    #[test]
    fn file_dialog_cancel_picks_nothing() {
        let support = TestHost::with_pick(None);
        let mut host = support.host;
        assert!(!host.open_file_dialog());
        assert!(support.app.borrow().load_paths.clone().is_empty());
        assert!(support.dialogs.errors().is_empty());
    }

    #[test]
    fn file_dialog_success_loads_picked_path() {
        let path = std::path::PathBuf::from("/tmp/game.dat");
        let support = TestHost::with_pick(Some(path.clone()));
        let mut host = support.host;
        assert!(host.open_file_dialog());
        assert_eq!(support.app.borrow().load_paths.clone(), [path]);
        assert!(support.dialogs.errors().is_empty());
    }

    #[test]
    fn file_dialog_failure_surfaces_error() {
        let path = std::path::PathBuf::from("/tmp/broken.dat");
        let support = TestHost::with_failing_pick(Some(path.clone()));
        let mut host = support.host;
        assert!(!host.open_file_dialog());
        assert_eq!(support.app.borrow().load_paths.clone(), [path]);
        assert_eq!(support.dialogs.errors(), ["fake load failed"]);
    }

    #[test]
    fn guard_blocks_non_quit() {
        let support = TestHost::new();
        let open = support.menu_ref(M::Open);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(open, &opener), HostAction::None);
        assert!(opener.calls().is_empty());
        assert!(support.app.borrow().load_paths.clone().is_empty());
    }

    fn guard_blocked_host() -> TestHost {
        use crate::testkit::TestAux;
        use wakui_core::aux::OpenRequest;
        let support = TestHost::new();
        let mut host = support.host;
        let id = match host.reserve_aux_window(TestAux::Alpha) {
            Some(OpenRequest::Create(id)) => id,
            _ => panic!("reservation"),
        };
        assert!(host.prepare_widget(id));
        host.aux
            .insert_for_test(TestAux::Alpha, Box::new(StubAux::new()));
        assert!(host.core.modal_guard_active());
        TestHost {
            host,
            menu: support.menu,
            dialogs: support.dialogs,
            app: support.app,
        }
    }

    #[test]
    fn open_guard_blocks_menu_commands() {
        let support = guard_blocked_host();
        let commands = [
            M::Open,
            M::Close,
            M::AuxAlpha,
            M::AuxGamma,
            M::AuxBeta,
            M::Session(S::Toggle),
        ]
        .map(|command| support.menu_ref(command));
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        for command in commands {
            assert_eq!(host.on_menu_command(command, &opener), HostAction::None);
        }
        assert!(opener.calls().is_empty());
        assert!(support.app.borrow().load_paths.clone().is_empty());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn open_guard_lets_quit_through() {
        let support = guard_blocked_host();
        let quit = support.menu_ref(M::Quit);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(quit, &opener), HostAction::Exit);
        // Intent only: host teardown waits for the runtime's ordered close.
        assert!(!host.core.close_exited());
    }

    #[test]
    fn canonical_events_share_external_modal_guard_and_effects() {
        use crate::event::UserEvent;
        use wakui_core::focus::FocusEvent;
        use wakui_core::session::SessionEvent;
        let support = guard_blocked_host();
        let about = support.menu_ref(M::About);
        let select = support.menu_ref(M::Session(S::Select(42)));
        let quit = support.menu_ref(M::Quit);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        // The guard comes from the open widget alone: no suspension owner
        // is held, yet commands and keyboard input stay blocked.
        assert!(!host.core.owns_pause(crate::testkit::owners::GUARD));
        assert!(host.core.modal_guard_active());
        for event in [
            UserEvent::Menu(about),
            UserEvent::Session(SessionEvent::Command(select)),
            UserEvent::Session(SessionEvent::Keyboard {
                key: wakui_core::Key::F4,
                pressed: true,
            }),
        ] {
            assert_eq!(host.on_event(event, &opener), HostAction::None);
        }
        assert!(support.app.borrow().active_item_id().is_none());
        assert!(support.menu.updates().is_empty());
        assert!(opener.calls().is_empty());
        assert_eq!(
            host.on_event(UserEvent::Focus(FocusEvent::Main(false)), &opener),
            HostAction::None
        );
        assert!(!host.core.main_active());
        assert_eq!(
            host.on_event(UserEvent::LiveTick, &opener),
            HostAction::None
        );
        assert_eq!(
            host.on_event(UserEvent::Menu(quit), &opener),
            HostAction::Exit
        );
        assert!(!host.core.close_exited());
    }

    #[test]
    fn keyboard_f4_effect_opens_native_aux() {
        use crate::event::UserEvent;
        use wakui_core::session::SessionEvent;
        let mut host = TestHost::new().host;
        let opener = FakeOpener::failing("no display");
        // The fake binds F4 to aux gamma, like the default view-side
        // binding. Production flavors never interpret keys.
        assert_eq!(
            host.on_event(
                UserEvent::Session(SessionEvent::Keyboard {
                    key: wakui_core::Key::F4,
                    pressed: true,
                }),
                &opener
            ),
            HostAction::None
        );
        assert_eq!(opener.calls(), [OpenerCall::Gamma]);
    }

    #[test]
    fn history_menu_records_open_attempt() {
        let support = TestHost::new();
        let history = support.menu_ref(M::AuxBeta);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(history, &opener), HostAction::None);
        assert_eq!(opener.calls(), [OpenerCall::Beta]);
        assert!(!host.aux.contains(crate::testkit::TestAux::Beta));
    }

    #[test]
    fn quit_exits_and_closes() {
        let support = TestHost::new();
        let quit = support.menu_ref(M::Quit);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(quit, &opener), HostAction::Exit);
        // Intent only: no teardown before the runtime's ordered close.
        assert!(!host.core.close_exited());
    }

    #[test]
    fn open_menu_routes_dialog_result() {
        let path = std::path::PathBuf::from("/tmp/game.dat");
        let support = TestHost::with_pick(Some(path.clone()));
        let open = support.menu_ref(M::Open);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(open, &opener), HostAction::FileLoaded);
        assert_eq!(support.app.borrow().load_paths.clone(), [path]);
    }

    #[test]
    fn close_menu_unloads_without_core() {
        let support = TestHost::new();
        let close = support.menu_ref(M::Close);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(close, &opener), HostAction::None);
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn session_menu_runs_command_and_syncs() {
        let support = TestHost::new();
        let toggle = support.menu_ref(M::Session(S::Toggle));
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(toggle, &opener), HostAction::None);
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn settings_menu_records_open_attempt() {
        let support = TestHost::new();
        let settings = support.menu_ref(M::AuxAlpha);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(settings, &opener), HostAction::None);
        assert_eq!(opener.calls(), [OpenerCall::Alpha]);
        assert!(host.aux.windows.ids().is_empty());
    }

    #[test]
    fn aux_gamma_menu_records_open_attempt() {
        let support = TestHost::new();
        let aux_gamma = support.menu_ref(M::AuxGamma);
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(host.on_menu_command(aux_gamma, &opener), HostAction::None);
        assert_eq!(opener.calls(), [OpenerCall::Gamma]);
        assert!(host.aux.windows.ids().is_empty());
    }

    #[test]
    fn session_menu_runs_every_command_without_core() {
        let support = TestHost::new();
        let activations = [
            S::Pause,
            S::Resume,
            S::Toggle,
            S::Restart,
            S::Create,
            S::SaveActiveOrNew,
            S::LoadActive,
            S::Select(7),
            S::Save(7),
            S::Load(7),
            S::Delete(7),
            S::Next,
            S::Previous,
        ]
        .map(|command| support.menu_ref(M::Session(command)));
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        let before = support.menu.updates().len();
        for command in activations {
            assert_eq!(host.on_menu_command(command, &opener), HostAction::None);
        }
        // dirty gate: identical chrome skips the IPC. Only the
        // first sync and the active-item selection change the sig.
        assert_eq!(support.menu.updates().len(), before + 2);
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn session_delegates_stay_total_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        let app = support.app.clone();
        assert!(!app.borrow().content_suspended());
        app.borrow_mut().pause_session();
        app.borrow_mut().resume_session();
        app.borrow_mut().toggle_pause_session();
        assert!(!app.borrow().content_suspended());
        assert!(!app.borrow_mut().save_active_item());
        assert!(!app.borrow_mut().load_active_item());
        app.borrow_mut().select_next();
        app.borrow_mut().select_previous();
        assert!(!app.borrow_mut().load_item(99));
        app.borrow_mut().save_item(99);
        app.borrow_mut().delete_item(99);
        app.borrow_mut().select_item(99);
        app.borrow_mut().create_item();
        app.borrow_mut().reset_session();
        host.core.unload();
        assert!(!app.borrow().content_loaded());
    }

    #[test]
    fn history_key_starts_changed_and_survives_failed_save() {
        let support = TestHost::new();
        let _host = support.host;
        assert!(support.app.borrow().log_changed());
    }

    #[test]
    fn apply_paths_stay_total_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        let _ = host.core.persist_fullscreen_default(true);
        assert!(!host.core.content_loaded());
    }

    #[test]
    fn error_modal_returns_without_blocking() {
        let support = TestHost::new();
        let mut host = support.host;
        host.core.report_host_error("test error".into());
        host.poll_notifications();
        // Bounded wait for the background ack; FakeDialogs answers instantly.
        let mut tries = 0;
        while support.dialogs.errors().is_empty() || host.core.dialog_pending() {
            host.poll_notifications();
            tries += 1;
            assert!(tries < 500, "modal ack never arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("test error"));
        assert_eq!(host.core.dialog_queue_len(), 0);
        assert!(!host.core.dialog_pending());
    }

    #[test]
    fn dead_dialog_thread_unwedges_the_queue() {
        use crate::testkit::owners;
        let support = TestHost::new();
        let mut host = support.host;
        host.core.set_pause_owner(owners::GUARD, true);
        // Simulate a dialog thread that died before acking: the
        // sender is dropped without sending (Disconnected).
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        host.core.report_host_error("disconnected dialog".into());
        let presentation = host
            .core
            .pump_dialog(std::time::Instant::now())
            .modal
            .unwrap();
        assert!(host.core.attach_dialog_receiver(presentation.token, rx));
        assert!(host.core.owns_pause(owners::MODAL));
        host.poll_notifications();
        // Reaped as a failed ack: flight gone, queue unblocked.
        assert!(!host.core.dialog_pending());
        assert_eq!(host.core.dialog_queue_len(), 0);
        assert!(!host.core.owns_pause(owners::MODAL));
        assert!(host.core.owns_pause(owners::GUARD));
    }

    #[test]
    fn disconnected_dialog_chains_through_native_presenter_before_modal_release() {
        use crate::testkit::owners;
        let support = TestHost::new();
        let mut host = support.host;
        host.core.set_pause_owner(owners::FOCUS, true);
        host.core.report_host_error("dead first dialog".into());
        let first = host
            .core
            .pump_dialog(std::time::Instant::now())
            .modal
            .unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(host.core.attach_dialog_receiver(first.token, receiver));
        host.core.report_host_error("chained native dialog".into());
        drop(sender);

        host.poll_notifications();
        assert_eq!(support.dialogs.errors().len(), 1);
        assert!(support.dialogs.errors()[0].contains("chained native dialog"));
        assert!(host.core.dialog_pending());
        assert!(host.core.dialog_flight_pending());
        assert!(host.core.owns_pause(owners::MODAL));
        assert!(host.core.owns_pause(owners::FOCUS));
        assert_eq!(host.core.dialog_queue_len(), 0);

        host.poll_notifications();
        assert!(!host.core.dialog_pending());
        assert!(!host.core.dialog_flight_pending());
        assert!(!host.core.owns_pause(owners::MODAL));
        assert!(host.core.owns_pause(owners::FOCUS));
        assert_eq!(support.dialogs.errors().len(), 1);
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
        assert_eq!(host.core.dialog_queue_len(), 0);
        assert!(support.app.borrow().log_changed());
    }

    #[test]
    fn keyboard_f4_routes_without_menu_sync() {
        use crate::testkit::TestAux;
        let support = TestHost::new();
        let mut host = support.host;
        host.apply_keyboard_f4();
        assert!(host.core.take_open_request(TestAux::Gamma));
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn session_verbs_stay_total_without_core() {
        let support = TestHost::new();
        let mut host = support.host;
        let app = support.app.clone();
        host.core.resume_content();
        app.borrow_mut().pause_session();
        host.core.clear_input();
        host.sync_fullscreen_from_settings();
        let _ = host.core.persist_fullscreen_default(false);
        assert!(!host.core.content_loaded());
    }
}
