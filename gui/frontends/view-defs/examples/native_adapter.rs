//! Executable integration sketch, not a WinUI/AppKit widget implementation.

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_frontend_viewdefs::{
    application::ApplicationHost, debugger::DebuggerEvent, frame::FrameCtx,
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{registry::SystemRegistry, session::SessionHandle};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize,
    renderer::{OsdLayer, OsdSink, StripSink},
};
use wakui::app::roles::{AuxPorts, DialogPorts, PollPorts as _};
use wakui::chrome::dialog::{ModalAdvance, ModalPresentation, ModalPresenter};
use wakui::chrome::menu::{
    CommandVisitor, MenuCapabilities, MenuId, MenuItem, MenuVisitor, Menubar, Submenu,
};
use wakui::chrome::menu_presenter::MenuPresenter;
use wakui::flow::loop_driver::{LoopHost, TurnLiveness, WakeNeed, drive_turn};
use wakui::input::commands::{Command, CommandRef};
use wakui::input::event::{CommandEffect, EventCommand, EventHandler, EventSink};

struct Common {
    app: ApplicationHost,
    debug: Option<wakui::chrome::aux::AuxId>,
}

impl Drop for Common {
    fn drop(&mut self) {
        self.app.begin_close();
        self.app.close_debug_flights();
        self.app.finish_close();
    }
}

struct NativeAdapter {
    common: Rc<RefCell<Common>>,
    redraws: usize, // Native resource/effect bookkeeping, not controller state.
    modals: usize,
    menus: MenuPresenter,
    menu_items: usize,
}

// Replace this visitor's operations with native menu APIs, not a second menu
// definition. Every operation may synchronously call back into Common.
struct NativeMenus {
    common: Rc<RefCell<Common>>,
    depth: usize,
    items: usize,
    declarations: usize,
}

impl CommandVisitor for NativeMenus {
    fn command(&mut self, command: &Command) {
        assert!(self.common.try_borrow_mut().is_ok());
        assert_eq!(self.depth, 0);
        assert!(!command.spec.debug_name().is_empty());
        self.declarations += 1;
    }
}

impl MenuVisitor for NativeMenus {
    fn begin_menubar(&mut self, menubar: &Menubar, _rebuild: bool) {
        assert!(self.common.try_borrow_mut().is_ok());
        assert_eq!(self.depth, 0);
        assert!(menubar.application_menu.is_some());
    }
    fn begin_menu(&mut self, _menu: &Submenu, _rebuild: bool) {
        self.depth += 1;
    }
    fn item(&mut self, _item: &MenuItem) {
        assert!(self.common.try_borrow_mut().is_ok());
        self.items += 1;
    }
    fn end_menu(&mut self, _id: MenuId) {
        self.depth -= 1;
    }
    fn end_menubar(&mut self) {
        assert_eq!(self.depth, 0);
    }
}

// A real adapter enqueues on its native loop. This Rc mailbox deliberately has
// no Send/Sync bound, just like a GTK callback transport.
struct NativeEvents(Rc<RefCell<Vec<EventCommand>>>);

impl EventSink for NativeEvents {
    fn send(&self, event: EventCommand) -> bool {
        self.0.borrow_mut().push(event);
        true
    }
}

struct NativeOsd(Rc<RefCell<Common>>);

impl std::fmt::Debug for NativeOsd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeOsd").finish_non_exhaustive()
    }
}

impl OsdSink for NativeOsd {
    fn set_osd_layer(&mut self, _layer: Option<OsdLayer>) {
        assert!(self.0.try_borrow_mut().is_ok());
    }
}

impl StripSink for NativeOsd {
    fn set_reserved_strip(&mut self, _strip: ReservedStrip) {
        assert!(self.0.try_borrow_mut().is_ok());
    }
}

// Real adapters create a dialog and wire its response callback here. Deliberate
// synchronous responses prove no controller borrow spans native presentation.
struct NativeModals {
    common: Rc<RefCell<Common>>,
    presented: std::cell::Cell<usize>,
}

impl ModalPresenter for NativeModals {
    fn present_modal(
        &self,
        presentation: ModalPresentation,
    ) -> Option<std::sync::mpsc::Receiver<()>> {
        let mut next = Some(presentation);
        while let Some(effect) = next {
            println!("native modal: {}", effect.detail);
            self.presented.set(self.presented.get() + 1);
            let advance = {
                let mut state = self.common.borrow_mut();
                assert!(state.app.pause_held());
                assert!(state.app.osd_modal_pending());
                let advance = state.app.acknowledge_osd_modal(effect.token);
                assert_eq!(
                    state.app.acknowledge_osd_modal(effect.token),
                    ModalAdvance::Ignored
                );
                advance
            };
            next = match advance {
                ModalAdvance::Present(effect) => Some(effect),
                ModalAdvance::Drained | ModalAdvance::Ignored => None,
            };
        }
        None // GTK-style callback delivery; workers return Some(receiver).
    }
}

impl NativeAdapter {
    fn deliver_modals(&mut self, effect: Option<ModalPresentation>) {
        if let Some(effect) = effect {
            let native = NativeModals {
                common: self.common.clone(),
                presented: std::cell::Cell::new(0),
            };
            let token = effect.token;
            let port: &dyn ModalPresenter = &native;
            if let Some(receiver) = port.present_modal(effect) {
                self.common
                    .borrow_mut()
                    .app
                    .attach_osd_modal_receiver(token, receiver);
            }
            self.modals += native.presented.get();
        }
    }
}

impl LoopHost for NativeAdapter {
    type Frame = FrameCtx;
    fn before_turn(&mut self) {
        self.common.borrow_mut().app.tick_focus_sync();
    }
    fn frame_ctx(&mut self) -> FrameCtx {
        self.common.borrow_mut().app.frame_ctx()
    }
    fn chrome(&mut self, ctx: &FrameCtx) {
        let (title, menu) = {
            let mut state = self.common.borrow_mut();
            (
                state.app.title_update(&ctx.tick()),
                state.app.menubar_update(
                    ctx,
                    MenuCapabilities {
                        about: false,
                        dialog_ellipsis: false,
                        application_menu: true,
                    },
                ),
            )
        };
        if let Some(title) = title {
            println!("native title: {title}");
        }
        if let Some(menu) = menu {
            let mut next = self.menus.submit(menu.menubar);
            let mut visitor = NativeMenus {
                common: self.common.clone(),
                depth: 0,
                items: 0,
                declarations: 0,
            };
            while let Some(plan) = next {
                let port: &mut dyn MenuVisitor = &mut visitor;
                plan.visit(port);
                next = self.menus.complete(plan);
            }
            self.menu_items += visitor.items;
            assert!(visitor.declarations > 0);
        }
    }
    fn pump_widgets(&mut self) -> WakeNeed {
        let mut state = self.common.borrow_mut();
        // The polled widget is a generic widget: refresh the cached snapshot,
        // then render the projected screen like any other aux window.
        if let Some(id) = state.app.polled_widget() {
            state.app.widget_refresh(id);
            if let Some(screen) = state.app.widget_screen(id) {
                println!("native debugger: {}", screen.title);
            }
            return state.app.poll_widget(id);
        }
        WakeNeed::default()
    }
    fn pump_dialog(&mut self, ctx: &FrameCtx) -> WakeNeed {
        let now = Instant::now();
        let turn = self.common.borrow_mut().app.pump_dialog(now);
        self.deliver_modals(turn.modal);
        let update =
            self.common
                .borrow_mut()
                .app
                .prepare_osd(SurfaceSize::new(320, 240), 480, ctx, now);
        let mut native = NativeOsd(self.common.clone());
        update.apply(&mut native);
        let state = self.common.borrow();
        WakeNeed {
            overlay_deadline: state.app.osd_wakeup(Instant::now()),
            flight_pending: state.app.osd_flight_pending(),
            ..Default::default()
        }
    }
    fn pump_aux(&mut self) -> WakeNeed {
        WakeNeed::default() // No auxiliary native resources in this sketch.
    }
    fn late_liveness(&mut self) -> TurnLiveness {
        let state = self.common.borrow();
        let tick = state.app.tick(); // Fresh post-ACK state, not the frame capture.
        TurnLiveness {
            present: tick.loaded,
            suspended: tick.paused,
            main_active: state.app.main_active(),
            live_follow: false,
        }
    }
    fn request_redraw(&mut self) {
        self.redraws += 1;
    }
}
struct TempSettings(PathBuf);
impl TempSettings {
    fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "nerust-native-adapter-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}
impl Drop for TempSettings {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Guard exists before session construction and outlives session flush/drop.
    let root = TempSettings::new()?;
    let session = SessionHandle::new_with_settings_paths(
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: false,
                supports_scaling: false,
            },
            presentation: None,
        },
        Arc::new(SystemRegistry::new(vec![])),
        Arc::new(AudioBackendRegistry::new()),
        SettingsPaths::from_root(root.0.clone()),
    )?;
    let mut native = NativeAdapter {
        common: Rc::new(RefCell::new(Common {
            app: ApplicationHost::new(session),
            debug: None,
        })),
        redraws: 0,
        modals: 0,
        menus: MenuPresenter::default(),
        menu_items: 0,
    };
    // Open the debugger widget through the generic aux boundary.
    let mut windows =
        wakui::chrome::aux::AuxWindows::<nerust_frontend_viewdefs::focus::AuxKind, u64, ()>::new();
    let id: Option<wakui::chrome::aux::AuxId> = {
        let mut common = native.common.borrow_mut();
        match common
            .app
            .reserve_widget(
                &mut windows,
                nerust_frontend_viewdefs::focus::AuxKind::Debug,
            )
            .expect("reservation")
        {
            Some(wakui::chrome::aux::OpenRequest::Create(id)) => {
                assert!(
                    common
                        .app
                        .prepare_widget(&mut windows, id)
                        .expect("prepared")
                );
                assert!(windows.complete_open(id, Some(1), ()).is_ok());
                Some(id)
            }
            _ => None,
        }
    };
    native.common.borrow_mut().debug = id;
    // Build chrome once so the resolution table holds the latest model,
    // like a native menu built before the first click.
    drive_turn(&mut native);
    let events = NativeEvents(Rc::new(RefCell::new(Vec::new())));
    let sink: &dyn EventSink = &events;
    // Opaque activation from the latest menubar, like a native menu callback.
    let open: CommandRef = {
        let common = native.common.borrow();
        common
            .app
            .menu_commands()
            .activation(nerust_frontend_viewdefs::commands::MenuCommand::Open)
            .expect("open activation")
    };
    assert!(sink.send(EventCommand::Command(open)));
    let event = events.0.borrow_mut().pop().unwrap();
    let outcome = {
        let mut common = native.common.borrow_mut();
        let handler: &mut dyn EventHandler<nerust_frontend_viewdefs::focus::AuxKind> =
            &mut common.app;
        handler.handle_event(event)?
    };
    assert_eq!(outcome.effect, Some(CommandEffect::OpenFile));
    // Native presentation can synchronously reenter only after borrow release.
    assert!(native.common.try_borrow_mut().is_ok());
    native.common.borrow_mut().app.note_main_departure();
    let idle = drive_turn(&mut native);
    assert_eq!(idle.deadline, None);
    assert!(!idle.wants_redraw);
    assert!(native.menu_items > 0);
    let menu_items = native.menu_items;
    {
        let mut state = native.common.borrow_mut();
        let handle = state
            .app
            .debug_handle_for(state.debug.expect("debug widget"))
            .unwrap();
        let effects = state
            .app
            .debug_gesture(handle, vec![], DebuggerEvent::RefreshPressed);
        assert!(!effects.open_ppu && !effects.epoch_reset);
        assert!(state.app.debug_drain_pending(handle)); // Dispatch queues; the next turn polls.
        state
            .app
            .report_host_error("Example host error (no native dialog created)".into());
    }
    drive_turn(&mut native); // Coreless debugger settles without waiting or sleeping.
    assert_eq!(native.modals, 1);
    {
        let state = native.common.borrow();
        let handle = state
            .app
            .debug_handle_for(state.debug.expect("debug widget"))
            .unwrap();
        assert!(!state.app.debug_drain_pending(handle));
        assert!(!state.app.tick().loaded);
        assert!(!state.app.osd_modal_pending());
        assert!(!state.app.pause_held());
    }
    native.common.borrow_mut().app.note_main_arrival();
    assert!(drive_turn(&mut native).wants_redraw);
    assert_eq!(native.redraws, 1);
    assert_eq!(
        native.menu_items, menu_items,
        "unchanged menus are not rebuilt"
    );
    println!(
        "verified: dyn events, command declarations, root visitor, idle turn, debugger poll, synchronous modal ACK, active redraw"
    );
    Ok(())
}
