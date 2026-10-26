//! macOS demo shell: real AppKit frontend over the shared host.
//!
//! `NSApplication` drives the loop; a 16ms `NSTimer` runs the shared
//! `drive_turn`, and `NSBitmapImageRep` presents the core framebuffer.
//! Menus, the debugger panel, the PPU viewer, and the settings sheet are
//! plain AppKit; every policy decision lives in `nerust_frontend_host`.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use block2::RcBlock;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_shell::{
    context::FrontendContext,
    registry::SystemRegistry,
    session::{access::SessionControl, commands::SessionCommand},
};
use nerust_mac::keys::{mac_key_to_nerust, selector};
use nerust_mac::{MacHost, MacSurface};
use objc2::{
    AnyThread, ClassType, MainThreadMarker, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, NSObject, Sel},
    sel,
};
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapFormat,
    NSBitmapImageRep, NSButton, NSEvent, NSEventMask, NSEventType, NSImage, NSImageView, NSMenu,
    NSMenuItem, NSModalResponseOK, NSOpenPanel, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub(crate) fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                eprintln!("macos: reentrant APP borrow dropped");
                return None;
            }
        };
        borrow.as_mut().map(f)
    })
}

pub fn with_dbg<R>(f: impl FnOnce(&mut super::dbg::DbgWin, &mut MacHost) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => return None,
        };
        let app = match borrow.as_mut() {
            Some(app) => app,
            None => return None,
        };
        let (dbg, host) = (&mut app.dbg, &mut app.host);
        dbg.as_mut().map(|dbg| f(dbg, host))
    })
}

pub fn with_settings<R>(
    f: impl FnOnce(&mut super::settings::SettingsDlg, &mut MacHost) -> R,
) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => return None,
        };
        let app = match borrow.as_mut() {
            Some(app) => app,
            None => return None,
        };
        let (settings, host) = (&mut app.settings, &mut app.host);
        settings.as_mut().map(|settings| f(settings, host))
    })
}

pub(crate) fn anyobject(delegate: &AppDelegate) -> &AnyObject {
    delegate.as_super().as_super()
}

/// Queued native effects for the UI tick: the shared host never touches
/// AppKit objects directly.
#[derive(Debug, Default)]
pub struct DemoSurface {
    pub titles: Vec<String>,
    pub fullscreen_calls: Vec<bool>,
    pub modal_errors: Vec<String>,
    pub debugger_requests: u32,
}

impl MacSurface for DemoSurface {
    fn set_title(&mut self, title: &str) {
        self.titles.push(title.to_string());
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen_calls.push(fullscreen);
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        None
    }

    fn show_error_modal(&mut self, detail: &str) {
        self.modal_errors.push(detail.to_string());
    }

    fn request_debugger(&mut self) {
        self.debugger_requests += 1;
    }
}

fn on_menu(selector: &str) {
    with_app(|app| {
        let mut surface = std::mem::take(&mut app.surface);
        app.host.on_menu(selector, &mut surface);
        app.surface = surface;
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotOp {
    Select,
    Save,
    Load,
    Delete,
}

pub(crate) struct StateMenuRefs {
    select: Retained<NSMenu>,
    save: Retained<NSMenu>,
    load: Retained<NSMenu>,
    delete: Retained<NSMenu>,
    pause: Retained<NSMenuItem>,
    resume: Retained<NSMenuItem>,
    reset: Retained<NSMenuItem>,
    create: Retained<NSMenuItem>,
    save_active: Retained<NSMenuItem>,
    load_active: Retained<NSMenuItem>,
    debugger_item: Retained<NSMenuItem>,
    history_item: Retained<NSMenuItem>,
    next: Retained<NSMenuItem>,
    prev: Retained<NSMenuItem>,
}

pub fn rgba_to_bgrx(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let n = out.len().min(rgba.len());
    for i in (0..n).step_by(4) {
        out[i] = rgba[i + 2];
        out[i + 1] = rgba[i + 1];
        out[i + 2] = rgba[i];
        out[i + 3] = 0;
    }
    out
}

// Shared `NSApplication` delegate: menu actions and the display-link
// timer land here, then route through the host. No policy lives here.
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NerustAppDelegate"]
    pub(crate) struct AppDelegate;

    impl AppDelegate {
        #[unsafe(method(onTick:))]
        fn on_tick(&self, _timer: Option<&NSTimer>) {
            autoreleasepool(|_| {
                with_app(|app| app.tick());
            });
        }

        #[unsafe(method(pauseEmulation:))]
        fn pause(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::PAUSE);
        }

        #[unsafe(method(resumeEmulation:))]
        fn resume(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::RESUME);
        }

        #[unsafe(method(resetEmulation:))]
        fn reset(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::RESET);
        }

        #[unsafe(method(createSaveSlot:))]
        fn create_slot(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::CREATE_SLOT);
        }

        #[unsafe(method(saveState:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::SAVE_SLOT);
        }

        #[unsafe(method(loadState:))]
        fn load(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::LOAD_SLOT);
        }

        #[unsafe(method(nextSlot:))]
        fn next_slot(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::NEXT_SLOT);
        }

        #[unsafe(method(prevSlot:))]
        fn prev_slot(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::PREV_SLOT);
        }

        #[unsafe(method(toggleDebugger:))]
        fn debugger(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::DEBUGGER);
        }

        #[unsafe(method(togglePpu:))]
        fn ppu(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_ppu());
        }

        #[unsafe(method(toggleFullscreen:))]
        fn fullscreen(&self, _sender: Option<&AnyObject>) {
            on_menu(selector::FULLSCREEN);
        }

        #[unsafe(method(openSettings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_settings());
        }

        #[unsafe(method(openFile:))]
        fn open_file(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_rom_file());
        }

        #[unsafe(method(quitApp:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.quit());
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _note: Option<&NSNotification>) {
            with_app(|app| app.host.on_active(true));
        }

        #[unsafe(method(applicationDidResignActive:))]
        fn did_resign_active(&self, _note: Option<&NSNotification>) {
            with_app(|app| app.host.on_active(false));
        }

        #[unsafe(method(debugPause:))]
        fn debug_pause(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                let _ = host.dispatch_cmd(C::Pause);
                let _ = dbg;
            });
        }

        #[unsafe(method(debugResume:))]
        fn debug_resume(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                let _ = host.dispatch_cmd(C::Resume);
                let _ = dbg;
            });
        }

        #[unsafe(method(debugStep:))]
        fn debug_step(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.step(host));
        }

        #[unsafe(method(submitSettings:))]
        fn submit_settings(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.submit_settings());
        }

        #[unsafe(method(cancelSettings:))]
        fn cancel_settings(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.cancel_settings());
        }

        #[unsafe(method(selectSlot:))]
        fn select_slot(&self, sender: Option<&AnyObject>) {
            with_app(|app| app.slot_action(SlotOp::Select, sender));
        }

        #[unsafe(method(saveSlot:))]
        fn save_slot(&self, sender: Option<&AnyObject>) {
            with_app(|app| app.slot_action(SlotOp::Save, sender));
        }

        #[unsafe(method(loadSlot:))]
        fn load_slot(&self, sender: Option<&AnyObject>) {
            with_app(|app| app.slot_action(SlotOp::Load, sender));
        }

        #[unsafe(method(deleteSlot:))]
        fn delete_slot(&self, sender: Option<&AnyObject>) {
            with_app(|app| app.slot_action(SlotOp::Delete, sender));
        }

        #[unsafe(method(openHistory:))]
        fn history(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_history());
        }

        #[unsafe(method(openAbout:))]
        fn about(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_about());
        }

        #[unsafe(method(closeFile:))]
        fn close_file(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_frontend_kernel::pause::PauseOwner;
                if !app.host.owns_pause(PauseOwner::Settings) {
                    SessionControl::unload(&mut app.host);
                }
            });
        }

        #[unsafe(method(debugStepFrame:))]
        fn debug_step_frame(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::StepFramePressed));
        }

        #[unsafe(method(debugRefresh:))]
        fn debug_refresh(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::RefreshPressed));
        }

        #[unsafe(method(debugFollow:))]
        fn debug_follow_toggle(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::FollowPcToggle));
        }

        #[unsafe(method(debugDisGo:))]
        fn debug_dis_go(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::DisGo));
        }

        #[unsafe(method(debugDisBack:))]
        fn debug_dis_back(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::DisBack));
        }

        #[unsafe(method(debugJump:))]
        fn debug_jump(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.follow_selected(host));
        }

        #[unsafe(method(debugSelectDis:))]
        fn debug_select_dis(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.select_dis_row(host));
        }

        #[unsafe(method(debugSpacePrev:))]
        fn debug_space_prev(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::SpacePrev));
        }

        #[unsafe(method(debugSpaceNext:))]
        fn debug_space_next(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::SpaceNext));
        }

        #[unsafe(method(debugPagePrev:))]
        fn debug_page_prev(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::MemPage(-1)));
        }

        #[unsafe(method(debugPageNext:))]
        fn debug_page_next(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::MemPage(1)));
        }

        #[unsafe(method(debugMemGo:))]
        fn debug_mem_go(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::MemGo));
        }

        #[unsafe(method(debugSelectMem:))]
        fn debug_select_mem(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.select_mem_row(host));
        }

        #[unsafe(method(debugRegSet:))]
        fn debug_reg_set(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::RegSet));
        }

        #[unsafe(method(debugRegSelect:))]
        fn debug_reg_select(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|button| button.tag())
                .unwrap_or(-1);
            with_dbg(|dbg, host| dbg.select_reg(host, tag));
        }

        #[unsafe(method(debugWatchSet:))]
        fn debug_watch_set(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::WatchSet));
        }

        #[unsafe(method(debugWatchClear:))]
        fn debug_watch_clear(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::WatchClear));
        }

        #[unsafe(method(debugFreezeSet:))]
        fn debug_freeze_set(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::FreezeSet));
        }

        #[unsafe(method(debugFreezeClear:))]
        fn debug_freeze_clear(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::FreezeClear));
        }

        #[unsafe(method(debugEditWrite:))]
        fn debug_edit_write(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::EditWrite));
        }

        #[unsafe(method(debugEditFreeze:))]
        fn debug_edit_freeze(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::EditFreezeSet));
        }

        #[unsafe(method(debugConfirmWrite:))]
        fn debug_confirm_write(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::ConfirmWrite));
        }

        #[unsafe(method(debugCancelWrite:))]
        fn debug_cancel_write(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::CancelWrite));
        }

        #[unsafe(method(debugBookmarkAdd:))]
        fn debug_bookmark_add(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.gesture(host, nerust_frontend_host::debugger::DebuggerEvent::BookmarkAdd));
        }

        #[unsafe(method(debugBookmarkJump:))]
        fn debug_bookmark_jump(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.bookmark_jump(host));
        }

        #[unsafe(method(debugBookmarkDelete:))]
        fn debug_bookmark_delete(&self, _sender: Option<&AnyObject>) {
            with_dbg(|dbg, host| dbg.bookmark_delete(host));
        }

        #[unsafe(method(settingsCombo:))]
        fn settings_combo(&self, sender: Option<&AnyObject>) {
            let (tag, index) = sender
                .and_then(|s| s.downcast_ref::<objc2_app_kit::NSComboBox>())
                .map(|combo| (combo.tag(), combo.indexOfSelectedItem()))
                .unwrap_or((0, -1));
            with_app(|app| app.settings_combo_changed(tag, index));
        }

        #[unsafe(method(settingsCheck:))]
        fn settings_check(&self, sender: Option<&AnyObject>) {
            let (tag, on) = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|button| (button.tag(), button.state() == objc2_app_kit::NSControlStateValueOn))
                .unwrap_or((0, false));
            with_app(|app| app.settings_check_changed(tag, on));
        }

        #[unsafe(method(settingsField:))]
        fn settings_field(&self, sender: Option<&AnyObject>) {
            let (tag, text) = sender
                .and_then(|s| s.downcast_ref::<objc2_app_kit::NSTextField>())
                .map(|field| (field.tag(), field.stringValue().to_string()))
                .unwrap_or((0, String::new()));
            with_app(|app| app.settings_field_committed(tag, text));
        }

        #[unsafe(method(settingsBindingChange:))]
        fn settings_binding_change(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|button| button.tag())
                .unwrap_or(0);
            with_app(|app| app.settings_binding_changed(tag, true));
        }

        #[unsafe(method(settingsBindingClear:))]
        fn settings_binding_clear(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|button| button.tag())
                .unwrap_or(0);
            with_app(|app| app.settings_binding_changed(tag, false));
        }

        #[unsafe(method(settingsBrowse:))]
        fn settings_browse(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.browse_storage_directory());
        }

        #[unsafe(method(historyFilter:))]
        fn history_filter(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.refresh_history());
        }

        #[unsafe(method(historyClear:))]
        fn history_clear(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.clear_history());
        }
    }
);

pub struct App {
    host: MacHost,
    surface: DemoSurface,
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    delegate: Retained<AppDelegate>,
    main: Retained<NSWindow>,
    image_view: Retained<NSImageView>,
    status_label: Retained<NSTextField>,
    toast_label: Retained<NSTextField>,
    rep: Option<Retained<NSBitmapImageRep>>,
    rep_size: (u32, u32),
    frames: u64,
    fps_at: Instant,
    fps: u32,
    dbg: Option<super::dbg::DbgWin>,
    ppu: Option<super::ppu::PpuWin>,
    settings: Option<super::settings::SettingsDlg>,
    history: Option<super::history::HistoryWin>,
    state_menu: Option<StateMenuRefs>,
    toast_seq: u64,
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let registry = Arc::new(SystemRegistry::new(vec![
        nerust_nes_factory::NesFactory::boxed().into(),
    ]));
    let audio = Arc::new(AudioBackendRegistry::new());
    let loader = registry
        .create_loader(Default::default())
        .expect("registry loader builds");
    let ctx = FrontendContext {
        gpu_factory: Rc::new(StubPipelineFactory),
        registry,
        rom_loader: loader,
        audio_registry: audio,
    };
    let host = MacHost::new(ctx).expect("session builds");

    let args: Vec<String> = std::env::args().collect();
    let rom = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .expect("usage: macos_demo <rom path>");
    let mut host = host;
    if let Err(detail) = host.load_path(&rom) {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&format!("ROM load failed: {detail}")));
        alert.runModal();
        std::process::exit(1);
    }

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let delegate: Retained<AppDelegate> = unsafe { msg_send![AppDelegate::class(), new] };
    let (main, image_view, status_label, toast_label) = build_main_window(mtm);
    build_menus(mtm, &app, &delegate);
    let state_menu = build_state_submenus(mtm, &app, &delegate);
    main.makeKeyAndOrderFront(None);
    let _timer: Retained<NSTimer> = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            1.0 / 63.0,
            delegate.as_super(),
            sel!(onTick:),
            None,
            true,
        )
    };

    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            host,
            surface: DemoSurface::default(),
            mtm,
            app: app.clone(),
            delegate: delegate.clone(),
            main,
            image_view,
            status_label,
            toast_label,
            rep: None,
            rep_size: (0, 0),
            frames: 0,
            fps_at: Instant::now(),
            fps: 0,
            dbg: None,
            ppu: None,
            settings: None,
            history: None,
            state_menu,
            toast_seq: 0,
        });
    });

    // Local key monitor: gameplay keys, settings capture, debugger keys.
    // Returning null swallows the event; otherwise it continues down the
    // responder chain (menu equivalents already resolved by NSApp).
    let monitor = RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        let handled = with_app(|app| {
            let event = unsafe { event.as_ref() };
            let code = event.keyCode();
            let down = event.r#type() == NSEventType::KeyDown;
            app.route_key(code, down)
        })
        .unwrap_or(false);
        if handled {
            std::ptr::null_mut()
        } else {
            event.as_ptr()
        }
    });
    let _token = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::KeyUp,
            &monitor,
        )
    };
    std::mem::forget(monitor);
    std::mem::forget(_token);

    // Verification hooks mirroring the Win32 twin: open aux windows without
    // GUI input, which headless runners cannot reliably deliver to menus.
    // drive_submit runs outside the APP borrow: it borrows APP itself
    // through with_settings.
    let args: Vec<String> = std::env::args().collect();
    with_app(|app| {
        use nerust_gui_shell::session::commands::SessionCommand as C;
        if args.iter().any(|a| a == "--pause") {
            app.host.dispatch_cmd(C::Pause);
        }
        if args.iter().any(|a| a == "--debugger") {
            app.open_debugger();
        }
        if args.iter().any(|a| a == "--ppu") {
            app.open_ppu();
        }
        if args
            .iter()
            .any(|a| a == "--settings" || a == "--settings-apply")
        {
            app.open_settings();
        }
    });
    if args.iter().any(|a| a == "--settings-apply") {
        let applied = super::settings::drive_submit();
        eprintln!("settings-apply applied={applied}");
    }
    if args.iter().any(|a| a == "--fullscreen") {
        with_app(|app| app.toggle_fullscreen());
    }
    // Modal common dialog: blocks the loop by design; path plumbing shares
    // host.load_path with the Win32 twin.
    if args.iter().any(|a| a == "--file-open") {
        with_app(|app| app.open_rom_file());
    }

    let _ = _timer;
    app.activate();
    app.run();
}

#[derive(Debug)]
struct StubPipelineFactory;

impl nerust_render_traits::renderer::PipelineFactory for StubPipelineFactory {
    fn create_pipeline(
        &self,
        _config: &nerust_render_traits::renderer::RendererConfig,
        _display: raw_window_handle::RawDisplayHandle,
    ) -> std::result::Result<
        Box<dyn nerust_render_traits::renderer::RenderPipeline>,
        nerust_render_traits::renderer::RendererError,
    > {
        Err(nerust_render_traits::renderer::RendererError::new(
            "macos demo needs no GPU pipeline",
            Box::new(std::io::Error::other("no GPU pipeline")),
        ))
    }
}

impl App {
    fn tick(&mut self) {
        self.host.turn();
        self.host.swap_frame_buffer();
        // Drive aux panels through the shared drain, mirroring the Win32
        // twin (each panel polls its own flight on the display-link tick).
        // Disjoint field borrows: panels and host never alias.
        if let Some(dbg) = self.dbg.as_mut() {
            dbg.on_tick(&mut self.host);
        }
        if let Some(ppu) = self.ppu.as_mut() {
            ppu.on_tick(&mut self.host);
        }
        // Reap aux windows the user closed via the traffic lights. No
        // NSWindowDelegate is wired in this prototype, so visibility is
        // polled here; the Win32 twin reaps via IsWindow in the same spot.
        if self.dbg.as_ref().is_some_and(|dbg| !dbg.window.isVisible()) {
            self.dbg = None;
        }
        if self.ppu.as_ref().is_some_and(|ppu| !ppu.window.isVisible()) {
            self.ppu = None;
        }
        if self
            .settings
            .as_ref()
            .is_some_and(|dlg| !dlg.window.isVisible())
        {
            self.settings = None;
        }
        if self
            .history
            .as_ref()
            .is_some_and(|win| !win.window.isVisible())
        {
            self.history = None;
        }
        self.frames += 1;
        if self.fps_at.elapsed().as_secs() >= 1 {
            self.fps = self.frames as u32;
            self.frames = 0;
            self.fps_at = Instant::now();
            self.chrome_sync();
        }
        self.present();
        self.drain_surface();
    }

    fn chrome_sync(&mut self) {
        let tick = self.host.tick();
        let state = if !tick.loaded {
            "NO ROM"
        } else if tick.paused {
            "PAUSED"
        } else {
            "RUNNING"
        };
        let geom = self
            .host
            .session()
            .frame_buffer()
            .map(|fb| format!("{}x{} s{}", fb.width(), fb.height(), fb.stride()))
            .unwrap_or_else(|| "-".to_string());
        let line = format!("{} fps | {state} | {geom}", self.fps);
        self.main.setTitle(&NSString::from_str(&format!(
            "nerust — {} fps {state}",
            self.fps
        )));
        self.status_label.setStringValue(&NSString::from_str(&line));
        self.rebuild_slot_menus();
        self.poll_toast();
        if self.history.is_some() {
            self.refresh_history();
        }
    }

    fn poll_toast(&mut self) {
        let language = self
            .host
            .session()
            .settings_snapshot()
            .shared
            .general
            .language;
        let seq = self.host.session().notification_history_seq();
        if seq == self.toast_seq {
            return;
        }
        self.toast_seq = seq;
        let lines = self
            .host
            .session_mut()
            .notification_history_lines_cached(language)
            .to_vec();
        if let Some(last) = lines.last() {
            self.toast_label
                .setStringValue(&NSString::from_str(&format!(
                    "[{}] {}",
                    last.module.rsplit("::").next().unwrap_or(last.module),
                    last.text
                )));
        } else {
            self.toast_label.setStringValue(&NSString::from_str(""));
        }
    }

    fn rebuild_slot_menus(&mut self) {
        let Some(refs) = self.state_menu.as_ref() else {
            return;
        };
        let session = self.host.session();
        let slots = session.slots().to_vec();
        let active = session.active_slot_id();
        use nerust_frontend_kernel::pause::PauseOwner;
        let tick = self.host.tick();
        let loaded = tick.loaded;
        let chrome_locked = self.host.owns_pause(PauseOwner::Settings);
        for (menu, op) in [
            (&refs.select, SlotOp::Select),
            (&refs.save, SlotOp::Save),
            (&refs.load, SlotOp::Load),
            (&refs.delete, SlotOp::Delete),
        ] {
            menu.removeAllItems();
            if slots.is_empty() {
                let empty: Retained<NSMenuItem> = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        self.mtm.alloc::<NSMenuItem>(),
                        &NSString::from_str("(No save slots)"),
                        None,
                        &NSString::from_str(""),
                    )
                };
                empty.setEnabled(false);
                menu.addItem(&empty);
                continue;
            }
            for slot in &slots {
                let item: Retained<NSMenuItem> = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        self.mtm.alloc::<NSMenuItem>(),
                        &NSString::from_str(&slot_label(slot, active)),
                        Some(match op {
                            SlotOp::Select => sel!(selectSlot:),
                            SlotOp::Save => sel!(saveSlot:),
                            SlotOp::Load => sel!(loadSlot:),
                            SlotOp::Delete => sel!(deleteSlot:),
                        }),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    item.setTarget(Some(anyobject(&self.delegate)));
                }
                item.setTag(slot.slot_id as isize);
                item.setEnabled(loaded);
                menu.addItem(&item);
            }
        }
        refs.pause.setEnabled(loaded);
        refs.resume.setEnabled(loaded);
        refs.reset.setEnabled(loaded);
        refs.create.setEnabled(loaded);
        refs.save_active.setEnabled(loaded);
        refs.load_active.setEnabled(loaded && active.is_some());
        refs.debugger_item.setEnabled(!chrome_locked);
        refs.history_item.setEnabled(!chrome_locked);
        refs.next.setEnabled(loaded);
        refs.prev.setEnabled(loaded);
    }

    fn drain_surface(&mut self) {
        let surface = std::mem::take(&mut self.surface);
        for title in &surface.titles {
            self.main.setTitle(&NSString::from_str(title));
        }
        for _ in &surface.fullscreen_calls {
            self.main.toggleFullScreen(None);
        }
        for detail in &surface.modal_errors {
            let alert = NSAlert::new(self.mtm);
            alert.setMessageText(&NSString::from_str(detail));
            alert.runModal();
        }
        if surface.debugger_requests > 0 {
            self.open_debugger();
        }
        self.surface = surface;
    }

    fn present(&mut self) {
        let Some((w, h, bgrx)) = self.host_framebuffer() else {
            return;
        };
        if self.rep_size != (w, h) {
            self.rep = make_rep(self.mtm, w, h);
            self.rep_size = (w, h);
            if let Some(rep) = self.rep.as_ref() {
                let image = NSImage::initWithSize(
                    NSImage::alloc(),
                    NSSize::new(f64::from(w), f64::from(h)),
                );
                image.addRepresentation(rep);
                self.image_view.setImage(Some(&image));
            }
        }
        if let Some(rep) = self.rep.as_ref() {
            unsafe {
                let dst = rep.bitmapData();
                std::ptr::copy_nonoverlapping(bgrx.as_ptr(), dst, bgrx.len());
            }
            NSView::setNeedsDisplay(&self.image_view, true);
        }
    }

    fn host_framebuffer(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let session = self.host.session();
        let fb = session.frame_buffer()?;
        let (w, h) = (fb.width() as u32, fb.height() as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let bytes: &[u8] = fb.as_ref();
        let stride = fb.stride().max(w as usize);
        let pal = fb.palette_as_rgba8().unwrap_or([0x80; 256]);
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let idx = bytes.get(y * stride + x).copied().unwrap_or(0) as usize;
                let base = (idx * 4).min(252);
                let o = (y * w as usize + x) * 4;
                out[o] = pal[base + 2];
                out[o + 1] = pal[base + 1];
                out[o + 2] = pal[base];
                out[o + 3] = 0;
            }
        }
        Some((w, h, out))
    }

    pub fn open_debugger(&mut self) {
        if self.dbg.is_some() {
            return;
        }
        self.host
            .dispatch_cmd(nerust_gui_shell::session::commands::SessionCommand::Pause);
        self.dbg = super::dbg::DbgWin::open(self.mtm, &mut self.host, &self.delegate);
    }

    pub fn open_ppu(&mut self) {
        if self.ppu.is_some() {
            return;
        }
        self.ppu = super::ppu::PpuWin::open(self.mtm, &mut self.host);
    }

    pub fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }
        self.settings =
            super::settings::SettingsDlg::open(self.mtm, &mut self.host, &self.delegate);
    }

    // setAllowedFileTypes is deprecated for allowedContentTypes (macOS 11+,
    // needs uniform-type-identifiers); this prototype keeps the string API
    // for a wider deployment floor, like the Win32 twin keeps GetOpenFileNameW.
    #[allow(deprecated)]
    fn open_rom_file(&mut self) {
        let panel = NSOpenPanel::openPanel(self.mtm);
        panel.setCanChooseFiles(true);
        let types = NSArray::from_retained_slice(&[NSString::from_str("nes")]);
        panel.setAllowedFileTypes(Some(&types));
        if panel.runModal() == NSModalResponseOK
            && let Some(url) = panel.URLs().firstObject()
            && let Some(path) = url.path().map(|s| PathBuf::from(s.to_string()))
            && let Err(detail) = self.host.load_path(&path)
        {
            let alert = NSAlert::new(self.mtm);
            alert.setMessageText(&NSString::from_str(&detail));
            alert.runModal();
        }
    }

    fn toggle_fullscreen(&mut self) {
        self.main.toggleFullScreen(None);
    }

    fn quit(&self) {
        self.app.terminate(None);
    }

    pub fn open_history(&mut self) {
        if self.history.is_some() {
            return;
        }
        self.history = super::history::HistoryWin::open(self.mtm, &mut self.host, &self.delegate);
    }

    pub fn open_about(&self) {
        self.app.orderFrontStandardAboutPanel(None);
    }

    pub fn refresh_history(&mut self) {
        if let Some(win) = self.history.as_mut() {
            win.refresh(&mut self.host);
        }
    }

    pub fn clear_history(&mut self) {
        self.host.clear_history();
        self.refresh_history();
    }

    pub fn slot_action(&mut self, op: SlotOp, sender: Option<&AnyObject>) {
        use nerust_frontend_kernel::pause::PauseOwner;
        let tag = sender
            .and_then(|s| s.downcast_ref::<NSMenuItem>())
            .map(|item| item.tag())
            .unwrap_or(-1);
        if tag < 0 || self.host.owns_pause(PauseOwner::Settings) {
            return;
        }
        let id = tag as u64;
        self.host.dispatch_cmd(match op {
            SlotOp::Select => SessionCommand::SelectActiveSlot(id),
            SlotOp::Save => SessionCommand::SaveSlot(id),
            SlotOp::Load => SessionCommand::LoadSlot(id),
            SlotOp::Delete => SessionCommand::DeleteSlot(id),
        });
    }

    pub fn settings_combo_changed(&mut self, tag: isize, index: isize) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.combo_changed(tag, index.max(0) as usize);
        }
        self.rebuild_settings();
    }

    pub fn settings_check_changed(&mut self, tag: isize, on: bool) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.check_changed(tag, on);
        }
        self.rebuild_settings();
    }

    pub fn settings_field_committed(&mut self, tag: isize, text: String) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.field_committed(tag, &text);
        }
        self.rebuild_settings();
    }

    pub fn settings_binding_changed(&mut self, tag: isize, start: bool) {
        if let Some(dlg) = self.settings.as_mut() {
            if start {
                dlg.binding_change(tag);
            } else {
                dlg.binding_clear(tag);
            }
        }
        self.rebuild_settings();
    }

    pub fn browse_storage_directory(&mut self) {
        let mtm = self.mtm;
        if let Some(dlg) = self.settings.as_mut() {
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseDirectories(true);
            panel.setCanChooseFiles(false);
            if panel.runModal() == NSModalResponseOK
                && let Some(url) = panel.URLs().firstObject()
                && let Some(path) = url.path().map(|s| PathBuf::from(s.to_string()))
            {
                dlg.field_committed_storage(path);
            }
        }
        self.rebuild_settings();
    }

    pub fn submit_settings(&mut self) {
        let applied = match (&mut self.settings, &mut self.host) {
            (Some(dlg), host) => dlg.submit(host),
            _ => false,
        };
        if applied {
            if let Some(dlg) = self.settings.as_ref() {
                dlg.close();
            }
            self.settings = None;
        }
    }

    pub fn cancel_settings(&mut self) {
        if let (Some(dlg), host) = (&mut self.settings, &mut self.host) {
            dlg.cancel(host);
        }
        if let Some(dlg) = self.settings.as_ref() {
            dlg.close();
        }
        self.settings = None;
    }

    pub fn rebuild_settings(&mut self) {
        if let Some(dlg) = self.settings.as_mut() {
            let delegate = self.delegate.clone();
            dlg.build_pages(self.mtm, &delegate);
        }
    }

    pub fn is_settings_capturing(&self) -> bool {
        self.settings.as_ref().is_some_and(|dlg| dlg.is_capturing())
    }

    pub fn capture_key(&mut self, key: nerust_keyboard::Key) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.capture_key(key);
            let delegate = self.delegate.clone();
            dlg.build_pages(self.mtm, &delegate);
        }
    }

    /// Local-monitor key routing. Returns true when the event is consumed.
    pub fn route_key(&mut self, code: u16, down: bool) -> bool {
        let key_window = self.app.keyWindow();
        let is_main = key_window.as_ref().is_some_and(|window| {
            std::ptr::eq(
                Retained::as_ptr(window) as *const (),
                Retained::as_ptr(&self.main) as *const (),
            )
        });
        if is_main {
            return self.host.on_key(code, down);
        }
        let is_settings = self.settings.as_ref().is_some_and(|dlg| {
            key_window.as_ref().is_some_and(|window| {
                std::ptr::eq(
                    Retained::as_ptr(window) as *const (),
                    Retained::as_ptr(&dlg.window) as *const (),
                )
            })
        });
        if is_settings {
            if down
                && self.is_settings_capturing()
                && let Some(key) = mac_key_to_nerust(code)
            {
                self.capture_key(key);
                return true;
            }
            return false;
        }
        let is_dbg = self.dbg.as_ref().is_some_and(|dbg| {
            key_window.as_ref().is_some_and(|window| {
                std::ptr::eq(
                    Retained::as_ptr(window) as *const (),
                    Retained::as_ptr(&dbg.window) as *const (),
                )
            })
        });
        if is_dbg && down {
            use nerust_frontend_host::debugger::DebuggerEvent as E;
            let event = match code {
                97 => Some(E::StepFramePressed),
                98 => Some(E::StepInstrPressed),
                101 => Some(E::TogglePausePressed),
                _ => None,
            };
            if let Some(event) = event {
                with_dbg(|dbg, host| dbg.gesture(host, event));
                return true;
            }
        }
        false
    }
}

fn make_rep(mtm: MainThreadMarker, w: u32, h: u32) -> Option<Retained<NSBitmapImageRep>> {
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(mtm.alloc::<NSBitmapImageRep>(),
                std::ptr::null_mut(),
                w as isize,
                h as isize,
                8,
                4,
                true,
                false,
                NSCalibratedRGBColorSpace,
                NSBitmapFormat::AlphaFirst | NSBitmapFormat::ThirtyTwoBitLittleEndian,
                (w * 4) as isize,
                32,
            )
    }
}

fn build_main_window(
    mtm: MainThreadMarker,
) -> (
    Retained<NSWindow>,
    Retained<NSImageView>,
    Retained<NSTextField>,
    Retained<NSTextField>,
) {
    let frame = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(768.0, 744.0));
    let main: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc::<NSWindow>(),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    main.setTitle(&NSString::from_str("nerust macos demo"));
    main.center();
    let holder: Retained<NSView> = NSView::initWithFrame(
        mtm.alloc::<NSView>(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(768.0, 744.0)),
    );
    let image_view: Retained<NSImageView> = NSImageView::initWithFrame(
        mtm.alloc::<NSImageView>(),
        NSRect::new(NSPoint::new(0.0, 24.0), NSSize::new(768.0, 720.0)),
    );
    let status_label: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(4.0, 2.0), NSSize::new(500.0, 20.0)),
    );
    status_label.setStringValue(&NSString::from_str(""));
    status_label.setEditable(false);
    status_label.setBezeled(false);
    status_label.setDrawsBackground(false);
    let toast_label: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(508.0, 2.0), NSSize::new(256.0, 20.0)),
    );
    toast_label.setStringValue(&NSString::from_str(""));
    toast_label.setEditable(false);
    toast_label.setBezeled(false);
    toast_label.setDrawsBackground(false);
    holder.addSubview(&image_view);
    holder.addSubview(&status_label);
    holder.addSubview(&toast_label);
    main.setContentView(Some(&holder));
    (main, image_view, status_label, toast_label)
}

fn menu_item(
    mtm: MainThreadMarker,
    target: &AnyObject,
    label: &str,
    action: Option<Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    let item: Retained<NSMenuItem> = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc::<NSMenuItem>(),
            &NSString::from_str(label),
            action,
            &NSString::from_str(key),
        )
    };
    if action.is_some() {
        unsafe {
            item.setTarget(Some(target));
        }
    }
    item
}

fn build_menus(mtm: MainThreadMarker, app: &NSApplication, delegate: &AppDelegate) {
    let target: &AnyObject = anyobject(delegate);
    let bar = NSMenu::new(mtm);
    let top = |title: &str, items: Vec<Retained<NSMenuItem>>| {
        let menu = NSMenu::initWithTitle(mtm.alloc::<NSMenu>(), &NSString::from_str(title));
        for item in items {
            menu.addItem(&item);
        }
        let head = menu_item(mtm, target, title, None, "");
        head.setSubmenu(Some(&menu));
        bar.addItem(&head);
    };
    top(
        "File",
        vec![
            menu_item(mtm, target, "Open…", Some(sel!(openFile:)), "o"),
            menu_item(mtm, target, "Close", Some(sel!(closeFile:)), ""),
            menu_item(mtm, target, "Settings…", Some(sel!(openSettings:)), ","),
            menu_item(mtm, target, "Quit", Some(sel!(quitApp:)), "q"),
        ],
    );
    let mut emu = vec![
        menu_item(mtm, target, "Pause", Some(sel!(pauseEmulation:)), ""),
        menu_item(mtm, target, "Resume", Some(sel!(resumeEmulation:)), ""),
        menu_item(mtm, target, "Reset", Some(sel!(resetEmulation:)), ""),
    ];
    // Save States submenu lives here; per-slot children are rebuilt per tick.
    let state_menu =
        NSMenu::initWithTitle(mtm.alloc::<NSMenu>(), &NSString::from_str("Save States"));
    let state_head = menu_item(mtm, target, "Save States", None, "");
    state_head.setSubmenu(Some(&state_menu));
    emu.push(state_head);
    emu.push(menu_item(
        mtm,
        target,
        "Open Debugger (F4)",
        Some(sel!(toggleDebugger:)),
        "",
    ));
    emu.push(menu_item(
        mtm,
        target,
        "Notification History",
        Some(sel!(openHistory:)),
        "",
    ));
    top("Emulation", emu);
    top(
        "View",
        vec![
            menu_item(mtm, target, "Fullscreen", Some(sel!(toggleFullscreen:)), ""),
            menu_item(mtm, target, "Debugger", Some(sel!(toggleDebugger:)), ""),
            menu_item(mtm, target, "PPU viewer", Some(sel!(togglePpu:)), ""),
        ],
    );
    top(
        "Help",
        vec![menu_item(
            mtm,
            target,
            "About nerust",
            Some(sel!(openAbout:)),
            "",
        )],
    );
    // State top-level menu holds the dynamic slot submenus.
    let state_top = NSMenu::initWithTitle(mtm.alloc::<NSMenu>(), &NSString::from_str("State"));
    let state_top_head = menu_item(mtm, target, "State", None, "");
    state_top_head.setSubmenu(Some(&state_top));
    bar.addItem(&state_top_head);
    app.setMainMenu(Some(&bar));
}

/// Menus whose children change per tick (slot lists + enablement).
/// Called once; [`App::rebuild_slot_menus`] refreshes contents.
fn build_state_submenus(
    mtm: MainThreadMarker,
    app: &NSApplication,
    delegate: &AppDelegate,
) -> Option<StateMenuRefs> {
    let target: &AnyObject = anyobject(delegate);
    let bar = app.mainMenu()?;
    let find_top = |title: &str| -> Option<Retained<NSMenuItem>> {
        let count = bar.numberOfItems();
        (0..count)
            .filter_map(|i| bar.itemAtIndex(i))
            .find(|item| item.title().to_string() == title)
    };
    let state_top = find_top("State")?;
    let state_menu = state_top.submenu()?;
    let sub = |title: &str| {
        let head = menu_item(mtm, target, title, None, "");
        let menu = NSMenu::initWithTitle(mtm.alloc::<NSMenu>(), &NSString::from_str(title));
        head.setSubmenu(Some(&menu));
        state_menu.addItem(&head);
        menu
    };
    let select = sub("Select Active Slot");
    let save = sub("Save Slot");
    let load = sub("Load Slot");
    let delete = sub("Delete Slot");
    let statik = |label: &str, action: Sel, key: &str| {
        let item = menu_item(mtm, target, label, Some(action), key);
        state_menu.addItem(&item);
        item
    };
    use objc2::runtime::Sel;
    let create = statik("New save slot", sel!(createSaveSlot:), "");
    let save_active = statik("Save Active Slot (F5)", sel!(saveStateSlot:), "");
    let load_active = statik("Load Active Slot (F8)", sel!(loadStateSlot:), "");
    let next = statik("Next slot (F6)", sel!(nextSlot:), "");
    let prev = statik("Previous slot (F7)", sel!(prevSlot:), "");

    // Gated static items live under Emulation; find them for enablement.
    let emu = find_top("Emulation")?.submenu()?;
    let find_in = |menu: &NSMenu, title: &str| -> Option<Retained<NSMenuItem>> {
        let count = menu.numberOfItems();
        (0..count)
            .filter_map(|i| menu.itemAtIndex(i))
            .find(|item| item.title().to_string() == title)
    };
    Some(StateMenuRefs {
        select,
        save,
        load,
        delete,
        pause: find_in(&emu, "Pause")?,
        resume: find_in(&emu, "Resume")?,
        reset: find_in(&emu, "Reset")?,
        create,
        save_active,
        load_active,
        debugger_item: find_in(&emu, "Open Debugger (F4)")?,
        history_item: find_in(&emu, "Notification History")?,
        next,
        prev,
    })
}
