use super::*;
use crate::focus::AuxKind;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{registry::SystemRegistry, session::access::PauseControl};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) struct Fixture {
    pub core: ApplicationHost,
    loader: Box<dyn nerust_gui_shell::load::RomLoader>,
    pub(super) root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nerust-application-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let registry = Arc::new(SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
        ]));
        let loader = registry.create_loader(Default::default()).unwrap();
        let session = SessionHandle::new_with_settings_paths(
            HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
            registry,
            Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
            SettingsPaths::from_root(root.clone()),
        )
        .unwrap();
        Self {
            core: ApplicationHost::new(session),
            loader,
            root,
        }
    }
    pub fn load(&mut self) {
        let mut rom = vec![0u8; 16 + 16384 + 8192];
        rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
        rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
        for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
            rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
        }
        let path = self.root.join("loop.nes");
        std::fs::write(&path, rom).unwrap();
        self.core.load_path(self.loader.as_mut(), &path).unwrap();
        assert!(self.core.tick().loaded);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.core.begin_close();
        self.core.close_debug_flights();
        self.core.finish_close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn focus_modal_and_explicit_pause_compose_across_actual_media_replacement() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.set_pause_owner(pause_owners::MODAL, true);
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    assert!(fixture.core.owns_pause(pause_owners::FOCUS));
    fixture.load();
    assert!(fixture.core.tick().paused);
    assert!(fixture.core.note_main_arrival());
    assert!(fixture.core.tick().paused, "modal still owns pause");
    fixture.core.pause();
    fixture.core.set_pause_owner(pause_owners::MODAL, false);
    assert!(fixture.core.tick().paused, "explicit pause survives ACK");
    fixture.core.resume();
    assert!(!fixture.core.tick().paused);
}

#[test]
fn settings_cancel_and_apply_preserve_modal_focus_and_user_owners() {
    let mut fixture = Fixture::new();
    assert!(fixture.core.note_settings_opened());
    fixture.load();
    assert!(fixture.core.tick().paused);
    assert_eq!(fixture.core.tick().frame_counter, 0);
    assert!(!fixture.core.note_settings_opened());
    fixture.core.set_pause_owner(pause_owners::MODAL, true);
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    let settings = fixture.core.settings_snapshot().clone();
    fixture.core.apply_settings(settings).unwrap();
    fixture.load();
    fixture.core.note_settings_dismissed();
    assert!(!fixture.core.owns_pause(pause_owners::SETTINGS));
    assert!(fixture.core.owns_pause(pause_owners::MODAL));
    assert!(fixture.core.owns_pause(pause_owners::FOCUS));
    fixture.core.pause();
    fixture.core.focus_event(FocusEvent::Main(true));
    fixture.core.set_pause_owner(pause_owners::MODAL, false);
    assert!(fixture.core.tick().paused);
    fixture.core.resume();
    assert!(!fixture.core.tick().paused);
    assert!(!fixture.core.note_settings_dismissed());
}

#[test]
fn session_verbs_stay_total_headless() {
    let mut fixture = Fixture::new();
    fixture.core.pause();
    fixture.core.resume_session();
    assert!(!fixture.core.tick().loaded);
    assert!(!fixture.core.pause_held());
}

#[test]
fn readonly_aux_arrival_cancels_pending_without_resuming_for_main_arrival() {
    use wakui::input::focus::FocusEvent;
    let mut fixture = Fixture::new();
    fixture.load();
    // Read-only arrival cancels the armed departure: nothing suspends.
    fixture.core.note_main_departure();
    fixture.core.focus_event(FocusEvent::Aux {
        resume: false,
        focused: true,
    });
    fixture.core.tick_focus_sync();
    assert!(!fixture.core.pause_held());
    // A later real departure still suspends.
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    assert!(fixture.core.owns_pause(pause_owners::FOCUS));
    // Read-only arrival over the held focus pause keeps it for main arrival.
    fixture.core.focus_event(FocusEvent::Aux {
        resume: false,
        focused: true,
    });
    assert!(fixture.core.owns_pause(pause_owners::FOCUS));
    assert!(fixture.core.note_main_arrival());
    assert!(!fixture.core.pause_held());
}

#[test]
fn unarmed_burst_end_commits_nothing() {
    let mut fixture = Fixture::new();
    fixture.core.tick_focus_sync();
    assert!(!fixture.core.note_main_arrival());
    assert!(!fixture.core.pause_held());
}

#[test]
fn armed_burst_end_without_core_stays_paused_off() {
    let mut fixture = Fixture::new();
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    assert!(!fixture.core.note_main_arrival());
    assert!(!fixture.core.pause_held());
}

#[test]
fn focus_effects_commit_before_capture_and_modal_ack_changes_late_truth() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    let captured = fixture.core.frame_ctx();
    assert!(captured.tick().paused);
    fixture.core.set_pause_owner(pause_owners::MODAL, true);
    fixture.core.focus_event(FocusEvent::Aux {
        resume: AuxKind::Debug.resume_on_arrival(),
        focused: true,
    });
    assert!(fixture.core.tick().paused);
    fixture.core.set_pause_owner(pause_owners::MODAL, false);
    assert!(!fixture.core.tick().paused);
    assert!(captured.tick().paused, "old capture is not liveness truth");
}

#[test]
fn application_close_flushes_exactly_once_after_resource_teardown() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.request_open(AuxKind::Debug);
    fixture.core.request_open(AuxKind::Ppu);
    fixture.core.set_live_wanted(true);
    let flushes = std::cell::Cell::new(0);
    let flush = |session: &mut SessionHandle| {
        flushes.set(flushes.get() + 1);
        session.flush_before_exit();
    };
    fixture.core.finish_close_with(flush);
    assert!(!fixture.core.flushed, "flush waits for close entry");
    assert!(fixture.core.begin_close());
    assert!(!fixture.core.window_usable());
    fixture.core.close_debug_flights();
    assert!(!fixture.core.take_open_request(AuxKind::Debug));
    assert!(!fixture.core.take_open_request(AuxKind::Ppu));
    assert!(!fixture.core.live_armed().load(Ordering::Acquire));
    assert!(!fixture.core.debug_pending());
    fixture.core.finish_close_with(flush);
    assert!(fixture.core.flushed);
    assert_eq!(flushes.get(), 1);
    // Deliberately re-arm state: a duplicate close must not run the teardown again.
    fixture.core.set_pause_owner(pause_owners::MODAL, true);
    assert!(!fixture.core.begin_close());
    fixture.core.finish_close_with(flush);
    assert_eq!(flushes.get(), 1);
    assert!(fixture.core.owns_pause(pause_owners::MODAL));
}

#[test]
fn chrome_gates_skip_repeat_pushes_and_retain_source_allocations() {
    let mut fixture = Fixture::new();
    let first = fixture.core.frame_ctx();
    let sig = fixture.core.menu_update(&first, false).unwrap();
    assert!(!sig.loaded);
    assert!(fixture.core.menu_update(&first, false).is_none());
    assert!(Arc::ptr_eq(&sig.shortcuts, first.shortcuts()));
    assert!(
        fixture
            .core
            .menu_update(&first, true)
            .unwrap()
            .settings_open
    );
    let replacement: Arc<[_]> = first.shortcuts().to_vec().into();
    assert!(!Arc::ptr_eq(&replacement, &sig.shortcuts));
    assert!(!fixture.core.title_same("nerust"));
    let title = fixture.core.title_update(&first.tick()).unwrap();
    assert_eq!(fixture.core.main_window().title, title);
    assert!(fixture.core.title_same(&title));
    assert!(fixture.core.title_update(&first.tick()).is_none());
    assert_eq!(fixture.core.frame_ctx().generation(), first.generation());
    let mut settings = fixture.core.settings_snapshot().clone();
    settings.local.notifications.info_duration_ms += 1;
    fixture.core.apply_settings(settings).unwrap();
    let replaced = fixture.core.frame_ctx();
    assert!(!Arc::ptr_eq(first.shortcuts(), replaced.shortcuts()));
    assert!(fixture.core.menu_update(&replaced, true).is_some());
}

#[test]
fn history_gate_tracks_sequence_language_and_reopen_without_native_state() {
    use nerust_settings_core::model::language::AppLanguage;
    let mut fixture = Fixture::new();
    assert!(fixture.core.history_key().is_none());
    assert!(fixture.core.history_update(AppLanguage::English).is_some());
    assert!(fixture.core.history_update(AppLanguage::English).is_none());
    fixture.core.report_host_error("history entry".into());
    assert!(
        !fixture
            .core
            .history_update(AppLanguage::English)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.core.history_update(AppLanguage::Japanese).is_some());
    fixture.core.clear_history();
    assert!(
        fixture
            .core
            .history_update(AppLanguage::Japanese)
            .unwrap()
            .is_empty()
    );
    fixture.core.invalidate_history();
    assert!(fixture.core.history_update(AppLanguage::Japanese).is_some());
    fixture.core.begin_close();
    assert!(fixture.core.history_key().is_none());
}

#[test]
fn render_retry_and_notification_pause_are_owned_by_application() {
    let mut fixture = Fixture::new();
    assert!(fixture.core.render_ready());
    fixture.core.render_failed("test render failure".into());
    assert!(!fixture.core.render_ready());
    let deadline = fixture.core.render_retry_deadline().unwrap();
    assert!(
        !fixture
            .core
            .take_render_retry(deadline - std::time::Duration::from_nanos(1))
    );
    assert!(fixture.core.take_render_retry(deadline));
    fixture.core.render_succeeded();
    assert!(fixture.core.render_ready());
    let mut notifications = OsdNotifications::new();
    let ctx = fixture.core.frame_ctx();
    let report = fixture
        .core
        .pump_notifications(&mut notifications, &ctx, Instant::now());
    assert!(report.modal_queued);
    assert!(fixture.core.owns_pause(pause_owners::MODAL));
}

#[test]
fn menubar_gate_is_independent_and_observes_capabilities_and_owned_settings() {
    let mut fixture = Fixture::new();
    let caps = wakui::chrome::menu::MenuCapabilities {
        about: false,
        dialog_ellipsis: false,
        application_menu: false,
    };
    assert!(fixture.core.menubar().is_none());
    let ctx = fixture.core.frame_ctx();
    assert!(fixture.core.menu_update(&ctx, false).is_some());
    let first = fixture.core.menubar_update(&ctx, caps).unwrap();
    assert_eq!(fixture.core.menubar(), Some(&first.menubar));
    assert!(std::ptr::eq(
        fixture.core.menubar().unwrap(),
        fixture.core.main_window().menu.as_ref().unwrap()
    ));
    assert!(fixture.core.menubar_update(&ctx, caps).is_none());
    let app_caps = wakui::chrome::menu::MenuCapabilities {
        application_menu: true,
        ..caps
    };
    let next = fixture.core.menubar_update(&ctx, app_caps).unwrap();
    assert!(!next.signature.loaded);
    assert!(next.menubar.application_menu.is_some());
    assert!(fixture.core.menubar_update(&ctx, app_caps).is_none());
    assert!(fixture.core.note_settings_opened());
    let guarded = fixture.core.menubar_update(&ctx, app_caps).unwrap();
    assert!(guarded.signature.settings_open);
    assert!(fixture.core.note_settings_dismissed());
    assert!(fixture.core.menu_update(&ctx, false).is_none());
    let unguarded = fixture.core.menubar_update(&ctx, app_caps).unwrap();
    assert!(!unguarded.signature.settings_open);
    // Reallocated captures notify consumers, but equal trees do not repaint.
    let mut settings = fixture.core.settings_snapshot().clone();
    settings.local.notifications.info_duration_ms += 1;
    fixture.core.apply_settings(settings).unwrap();
    let ctx = fixture.core.frame_ctx();
    let replaced = fixture.core.menubar_update(&ctx, app_caps).unwrap();
    assert_eq!(replaced.menubar, unguarded.menubar);
    let presenter = wakui::chrome::menu_presenter::MenuPresenter::default();
    let plan = presenter.submit(unguarded.menubar).unwrap();
    assert!(presenter.complete(plan).is_none());
    assert!(presenter.submit(replaced.menubar).is_none());
    assert!(fixture.core.menubar_update(&ctx, app_caps).is_none());
}

#[test]
fn menubar_delivers_signature_only_changes_without_frontend_fallbacks() {
    let mut fixture = Fixture::new();
    let caps = wakui::chrome::menu::MenuCapabilities {
        about: false,
        dialog_ellipsis: true,
        application_menu: false,
    };
    let ctx = fixture.core.frame_ctx();
    let initial = fixture.core.menubar_update(&ctx, caps).unwrap();
    let select = fixture
        .core
        .menu_commands()
        .activation(crate::commands::MenuCommand::Session(
            nerust_gui_shell::session::commands::SessionCommand::SelectActiveSlot(73),
        ))
        .unwrap();
    fixture
        .core
        .dispatch(SessionEvent::Command(select))
        .unwrap();
    let update = fixture.core.menubar_update(&ctx, caps).unwrap();
    assert_eq!(update.signature.active_slot, Some(73));
    assert_eq!(initial.menubar, update.menubar);
    assert!(fixture.core.menubar_update(&ctx, caps).is_none());
}

#[test]
fn main_window_menu_event_creates_owned_debug_window_before_native_creation() {
    use crate::commands::MenuCommand;
    use wakui::chrome::aux::{AuxWindows, OpenRequest};
    use wakui::chrome::window::WindowVisitor;
    use wakui::{
        chrome::menu::MenuEntry,
        chrome::window::{WidgetContents, Window},
        input::event::{CommandEffect, EventCommand, EventHandler},
    };

    let mut fixture = Fixture::new();
    let ctx = fixture.core.frame_ctx();
    fixture.core.menubar_update(
        &ctx,
        MenuCapabilities {
            about: false,
            dialog_ellipsis: false,
            application_menu: false,
        },
    );
    let debugger = fixture
        .core
        .menu_commands()
        .activation(MenuCommand::Debugger)
        .expect("debugger activation");
    let command = fixture
        .core
        .main_window()
        .menu
        .as_ref()
        .unwrap()
        .menus
        .iter()
        .flat_map(|menu| &menu.children)
        .find_map(|entry| match entry {
            MenuEntry::Item(item) if item.command == Some(debugger) => item.command,
            _ => None,
        })
        .expect("OpenDebugger belongs to this main window's actual menu");
    let outcome = fixture
        .core
        .handle_event(EventCommand::Command(command))
        .unwrap();
    let Some(CommandEffect::OpenAux(kind)) = outcome.effect else {
        panic!("expected auxiliary open");
    };
    assert_eq!(kind, AuxKind::Debug);
    let mut windows = AuxWindows::<AuxKind, u64, ()>::new();
    let OpenRequest::Create(id) = windows.request_open(kind, kind.title()) else {
        panic!("expected creation");
    };
    assert!(windows.get(kind).is_none(), "native creation has not run");
    assert!(matches!(
        windows.window(id).unwrap().contents,
        WidgetContents::PendingControls
    ));
    struct MenuVisitor(bool);
    impl WindowVisitor for MenuVisitor {
        fn begin_window(&mut self, _: &Window) {
            assert!(!self.0);
        }
        fn visit_menu(&mut self, _: &Menubar) {
            self.0 = true;
        }
        fn visit_main(&mut self, _: &wakui::chrome::window::MainContents) {
            assert!(self.0, "menu is visited before contents");
        }
    }
    let mut visitor = MenuVisitor(false);
    fixture.core.main_window().visit(&mut visitor);
    assert!(visitor.0);
}

#[test]
fn dyn_events_guard_settings_but_preserve_focus_clear_lifecycle_quit_and_tick() {
    use crate::commands::{MenuCommand as M, NativeRole};
    use nerust_gui_shell::session::commands::SessionCommand as S;
    use wakui::input::event::{CommandEffect as E, EventCommand as C, EventHandler};
    let mut fixture = Fixture::new();
    let ctx = fixture.core.frame_ctx();
    fixture
        .core
        .menubar_update(
            &ctx,
            wakui::chrome::menu::MenuCapabilities {
                about: true,
                dialog_ellipsis: false,
                application_menu: false,
            },
        )
        .unwrap();
    let table = fixture.core.menu_commands().clone();
    let menu = |command: M| C::Command(table.activation(command).unwrap());
    let session_command = |command: S| {
        C::Session(SessionEvent::Command(
            table.activation(M::Session(command)).unwrap(),
        ))
    };
    // The modal guard is view-side state: no guard flag crosses the boundary.
    assert!(fixture.core.note_settings_opened());
    let host: &mut dyn EventHandler<AuxKind> = &mut fixture.core;
    for event in [
        menu(M::Open),
        menu(M::Close),
        menu(M::Settings),
        menu(M::Debugger),
        menu(M::NotificationHistory),
        menu(M::Native(NativeRole::About)),
        menu(M::Session(S::Pause)),
        session_command(S::Resume),
        C::Session(SessionEvent::Keyboard {
            key: wakui::Key::F2,
            pressed: true,
        }),
    ] {
        let result = host.handle_event(event).unwrap();
        assert!(!result.handled && result.effect.is_none() && result.session.command.is_none());
    }
    let quit = host
        .handle_event(menu(M::Native(NativeRole::Quit)))
        .unwrap();
    assert_eq!(quit.effect, Some(E::Quit));
    let tick = host.handle_event(C::LiveTick).unwrap();
    assert_eq!(tick.effect, Some(E::LiveTick));
    let focus = host
        .handle_event(C::Focus(FocusEvent::Main(false)))
        .unwrap();
    assert!(focus.handled);
    let clear = host
        .handle_event(C::Session(SessionEvent::ClearInput))
        .unwrap();
    assert!(clear.handled);
    let guarded = host.handle_event(menu(M::Debugger)).unwrap();
    assert!(!guarded.handled, "owned Settings also guards");
    assert!(fixture.core.note_settings_dismissed());
    assert!(!fixture.core.main_active());
    assert!(!fixture.core.pause_held());
}

#[test]
fn event_effects_are_owned_and_native_reentry_needs_no_host_borrow() {
    use crate::commands::{MenuCommand as M, NativeRole};
    use std::{cell::RefCell, rc::Rc};
    use wakui::input::event::EventSink;
    use wakui::input::event::{CommandEffect as E, EventCommand as C, EventHandler};
    struct Sink(Rc<RefCell<Vec<C>>>);
    impl EventSink for Sink {
        fn send(&self, event: C) -> bool {
            self.0.borrow_mut().push(event);
            true
        }
    }
    let fixture = RefCell::new(Fixture::new());
    let sink = Sink(Rc::new(RefCell::new(Vec::new())));
    let transport: &dyn EventSink = &sink;
    let table = {
        let mut state = fixture.borrow_mut();
        let ctx = state.core.frame_ctx();
        state
            .core
            .menubar_update(
                &ctx,
                wakui::chrome::menu::MenuCapabilities {
                    about: true,
                    dialog_ellipsis: false,
                    application_menu: false,
                },
            )
            .unwrap();
        state.core.menu_commands().clone()
    };
    let menu = |command: M| C::Command(table.activation(command).unwrap());
    for (command, effect) in [
        (M::Open, E::OpenFile),
        (M::Settings, E::OpenAux(AuxKind::Settings)),
        (M::Debugger, E::OpenAux(AuxKind::Debug)),
        (M::NotificationHistory, E::OpenAux(AuxKind::History)),
        (M::Native(NativeRole::About), E::About),
        (M::Native(NativeRole::Quit), E::Quit),
    ] {
        assert!(transport.send(menu(command)));
        let event = sink.0.borrow_mut().pop().unwrap();
        let result = {
            let mut state = fixture.borrow_mut();
            let handler: &mut dyn EventHandler<AuxKind> = &mut state.core;
            handler.handle_event(event).unwrap()
        };
        assert_eq!(result.effect, Some(effect));
        // Simulate a synchronously reentrant native operation after dispatch.
        assert!(fixture.try_borrow_mut().is_ok());
    }
    let mut state = fixture.borrow_mut();
    // Shortcuts arrive as framework keyboard events; the shell binding
    // table resolves them view-side. Bind each action, then press F4.
    use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding};
    for (action, effect) in [
        (ShortcutAction::OpenDebugger, E::OpenAux(AuxKind::Debug)),
        (ShortcutAction::ToggleFullscreen, E::ToggleFullscreen),
    ] {
        let mut settings = state.core.settings_snapshot().clone();
        settings.shared.input.shortcuts.keyboard = vec![ShortcutBinding {
            action,
            key: Some(wakui::Key::F4),
        }];
        state.core.apply_settings(settings).unwrap();
        let result = state
            .core
            .handle_event(C::Session(SessionEvent::Keyboard {
                key: wakui::Key::F4,
                pressed: true,
            }))
            .unwrap();
        assert_eq!(result.effect, Some(effect));
        assert!(result.session.command.is_none());
        // Release before the next binding so the press is not a repeat.
        state
            .core
            .handle_event(C::Session(SessionEvent::Keyboard {
                key: wakui::Key::F4,
                pressed: false,
            }))
            .unwrap();
    }
    struct Dead;
    impl EventSink for Dead {
        fn send(&self, _: C) -> bool {
            false
        }
    }
    let dead: &dyn EventSink = &Dead;
    assert!(!dead.send(C::LiveTick));
}

#[test]
fn menu_events_execute_once_preserve_pause_owners_and_remain_total_coreless() {
    use crate::commands::MenuCommand as M;
    use nerust_gui_shell::session::commands::SessionCommand as S;
    use wakui::input::event::{EventCommand as C, EventHandler};
    let mut fixture = Fixture::new();
    let ctx = fixture.core.frame_ctx();
    fixture
        .core
        .menubar_update(
            &ctx,
            wakui::chrome::menu::MenuCapabilities {
                about: true,
                dialog_ellipsis: false,
                application_menu: false,
            },
        )
        .unwrap();
    let table = fixture.core.menu_commands().clone();
    let menu = |command: M| C::Command(table.activation(command).unwrap());
    fixture
        .core
        .handle_event(menu(M::Session(S::SelectActiveSlot(73))))
        .unwrap();
    assert_eq!(fixture.core.session().active_slot_id(), Some(73));
    let Err(error) = fixture.core.handle_event(menu(M::Session(S::Reset))) else {
        panic!("reset without a core must fail");
    };
    assert_eq!(error.0, "no emulation core active");
    let load = fixture
        .core
        .handle_event(menu(M::Session(S::LoadActiveSlot)))
        .unwrap();
    assert!(
        load.session.command.is_some(),
        "disabled coreless commands still dispatch"
    );
    fixture.load();
    fixture
        .core
        .handle_event(menu(M::Session(S::TogglePause)))
        .unwrap();
    assert!(
        fixture.core.tick().paused,
        "double dispatch would toggle back"
    );
    assert!(fixture.core.owns_pause(pause_owners::USER));
    fixture.core.set_pause_owner(pause_owners::MODAL, true);
    fixture
        .core
        .handle_event(menu(M::Session(S::Resume)))
        .unwrap();
    assert!(!fixture.core.owns_pause(pause_owners::USER));
    assert!(fixture.core.tick().paused, "modal still owns pause");
    fixture.core.set_pause_owner(pause_owners::MODAL, false);
    assert!(!fixture.core.tick().paused);
    fixture
        .core
        .handle_event(C::Focus(FocusEvent::Main(false)))
        .unwrap();
    fixture
        .core
        .handle_event(C::Focus(FocusEvent::BurstEnd))
        .unwrap();
    assert!(fixture.core.owns_pause(pause_owners::FOCUS));
    fixture
        .core
        .handle_event(C::Focus(FocusEvent::Main(true)))
        .unwrap();
    assert!(!fixture.core.tick().paused);
    fixture.core.handle_event(menu(M::Close)).unwrap();
    assert!(!fixture.core.tick().loaded);
}
