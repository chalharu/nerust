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
use nerust_frontend_host::{
    application::ApplicationHost,
    debugger::{DebuggerDriver, DebuggerEvent},
    frame::FrameCtx,
    loop_driver::{LoopHost, TurnLiveness, WakeNeed, drive_turn},
    menu::{MenuCapabilities, MenuId, MenuItem, MenuModel, MenuVisitor, Submenu},
    osd::{ModalAdvance, ModalPresentation, ModalToken, OsdNotifications},
    widgets::MenuPresenter,
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{registry::SystemRegistry, session::SessionHandle};

struct Common {
    app: ApplicationHost,
    osd: OsdNotifications,
    debugger: DebuggerDriver,
}

impl Drop for Common {
    fn drop(&mut self) {
        self.app.begin_close();
        self.osd.close();
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
}

impl MenuVisitor for NativeMenus {
    fn begin_model(&mut self, _rebuild: bool) {
        assert!(self.common.try_borrow_mut().is_ok());
        assert_eq!(self.depth, 0);
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
    fn end_model(&mut self) {
        assert_eq!(self.depth, 0);
    }
}

// A real adapter creates a dialog and wires its response callback here. This
// deliberately responds synchronously: no controller borrow may span this call.
fn present_modal(
    effect: ModalPresentation,
    on_response: impl FnOnce(ModalToken) -> ModalAdvance,
) -> ModalAdvance {
    println!("native modal: {}", effect.detail);
    on_response(effect.token)
}

impl NativeAdapter {
    fn deliver_modals(&mut self, mut next: Option<ModalPresentation>) {
        while let Some(effect) = next {
            let common = Rc::clone(&self.common);
            let advance = present_modal(effect, move |token| {
                // Reenter the same Rc<RefCell> from the native response callback.
                let mut state = common.borrow_mut();
                let Common { app, osd, .. } = &mut *state;
                assert!(app.pause_held());
                assert!(osd.take_modal().is_none(), "no overlapping dialogs");
                let advance = app.acknowledge_modal(osd, token);
                assert_eq!(app.acknowledge_modal(osd, token), ModalAdvance::Ignored);
                advance
            });
            self.modals += 1;
            // The callback borrow has ended before another dialog can be shown.
            next = match advance {
                ModalAdvance::Present(effect) => Some(effect),
                ModalAdvance::Drained | ModalAdvance::Ignored => None,
            };
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
                state.app.menu_update(ctx, false),
            )
        };
        if let Some(title) = title {
            println!("native title: {title}");
        }
        if let Some(menu) = menu {
            let mut next = self.menus.submit(MenuModel::build(
                &menu,
                MenuCapabilities {
                    about: false,
                    dialog_ellipsis: false,
                },
            ));
            let mut visitor = NativeMenus {
                common: self.common.clone(),
                depth: 0,
                items: 0,
            };
            while let Some(plan) = next {
                plan.visit(&mut visitor);
                next = self.menus.complete(plan);
            }
            self.menu_items += visitor.items;
        }
    }
    fn pump_debug(&mut self) -> WakeNeed {
        let (effects, view) = {
            let mut state = self.common.borrow_mut();
            let Common { app, debugger, .. } = &mut *state;
            let effects = debugger.poll(app);
            let view = effects.scope.map(|_| debugger.view_snapshot());
            (effects, view)
        };
        if let Some(view) = view {
            println!("native debugger: {:?}", view.display.status);
        }
        WakeNeed {
            flight_pending: effects.pending,
            ..Default::default()
        }
    }
    fn pump_osd(&mut self, ctx: &FrameCtx) -> WakeNeed {
        let effect = {
            let mut state = self.common.borrow_mut();
            let Common { app, osd, .. } = &mut *state;
            app.pump_notifications(osd, ctx, Instant::now());
            osd.take_modal()
        };
        self.deliver_modals(effect);
        let state = self.common.borrow();
        WakeNeed {
            osd_deadline: state.osd.wakeup_deadline(Instant::now()),
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
            loaded: tick.loaded,
            paused: tick.paused,
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
    let debugger = DebuggerDriver::from_session(&session);
    let mut native = NativeAdapter {
        common: Rc::new(RefCell::new(Common {
            app: ApplicationHost::new(session),
            osd: OsdNotifications::new(),
            debugger,
        })),
        redraws: 0,
        modals: 0,
        menus: MenuPresenter::default(),
        menu_items: 0,
    };
    native.common.borrow_mut().app.note_main_departure();
    let idle = drive_turn(&mut native);
    assert_eq!(idle.deadline, None);
    assert!(!idle.wants_redraw);
    assert!(native.menu_items > 0);
    let menu_items = native.menu_items;
    {
        let mut state = native.common.borrow_mut();
        let Common { app, debugger, .. } = &mut *state;
        let effects = debugger.dispatch(app.session(), [], DebuggerEvent::RefreshPressed, false);
        assert!(!effects.open_ppu && !effects.epoch_reset);
        assert!(debugger.pending()); // Dispatch queues; the next turn polls.
        app.report_host_error("Example host error (no native dialog created)".into());
    }
    drive_turn(&mut native); // Coreless debugger settles without waiting or sleeping.
    assert_eq!(native.modals, 1);
    {
        let state = native.common.borrow();
        assert!(!state.debugger.pending());
        assert!(!state.app.tick().loaded);
        assert!(!state.osd.modal_pending());
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
        "verified: menu visitor, idle turn, debugger poll, synchronous modal ACK, active redraw"
    );
    Ok(())
}
