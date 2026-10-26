//! macOS demo shell: real AppKit frontend over the shared host.
//!
//! `NSApplication` drives the loop; a 16ms `NSTimer` runs the shared
//! `drive_turn`, and `NSBitmapImageRep` presents the core framebuffer.
//! Menus, the debugger panel, the PPU viewer, and the settings sheet are
//! plain AppKit; every policy decision lives in `nerust_frontend_host`.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_shell::{context::FrontendContext, registry::SystemRegistry};
use nerust_mac::MacHost;
use objc2::{
    AnyThread, ClassType, MainThreadMarker, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, NSObject},
    sel,
};
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapFormat,
    NSBitmapImageRep, NSImage, NSImageView, NSMenu, NSMenuItem, NSModalResponseOK, NSOpenPanel,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSNotification, NSRect, NSSize, NSString, NSTimer};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
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
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::Pause);
            });
        }

        #[unsafe(method(resumeEmulation:))]
        fn resume(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::Resume);
            });
        }

        #[unsafe(method(resetEmulation:))]
        fn reset(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::Reset);
            });
        }

        #[unsafe(method(newSaveSlot:))]
        fn new_slot(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::CreateSlot);
            });
        }

        #[unsafe(method(saveState:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::SaveActiveSlotOrNew);
            });
        }

        #[unsafe(method(loadState:))]
        fn load(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::LoadActiveSlot);
            });
        }

        #[unsafe(method(nextSlot:))]
        fn next_slot(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::SelectNextSlot);
            });
        }

        #[unsafe(method(prevSlot:))]
        fn prev_slot(&self, _sender: Option<&AnyObject>) {
            with_app(|app| {
                use nerust_gui_shell::session::commands::SessionCommand as C;
                app.host.dispatch_cmd(C::SelectPreviousSlot);
            });
        }

        #[unsafe(method(toggleDebugger:))]
        fn debugger(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_debugger());
        }

        #[unsafe(method(togglePpu:))]
        fn ppu(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.open_ppu());
        }

        #[unsafe(method(toggleFullscreen:))]
        fn fullscreen(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.toggle_fullscreen());
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
            with_settings(|dlg, host| dlg.submit(host));
        }

        #[unsafe(method(cancelSettings:))]
        fn cancel_settings(&self, _sender: Option<&AnyObject>) {
            with_settings(|dlg, host| {
                dlg.cancel(host);
                dlg.close();
            });
        }
    }
);

pub struct App {
    host: MacHost,
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    delegate: Retained<AppDelegate>,
    main: Retained<NSWindow>,
    image_view: Retained<NSImageView>,
    rep: Option<Retained<NSBitmapImageRep>>,
    rep_size: (u32, u32),
    frames: u64,
    fps_at: Instant,
    fps: u32,
    dbg: Option<super::dbg::DbgWin>,
    ppu: Option<super::ppu::PpuWin>,
    settings: Option<super::settings::SettingsDlg>,
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
        .get(1)
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
    build_menus(mtm, &app, &delegate);

    let frame = NSRect::new(
        objc2_foundation::NSPoint::new(100.0, 100.0),
        NSSize::new(768.0, 720.0),
    );
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
    let image_view: Retained<NSImageView> = NSImageView::initWithFrame(
        mtm.alloc::<NSImageView>(),
        NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            NSSize::new(768.0, 720.0),
        ),
    );
    main.setContentView(Some(&image_view));
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
            mtm,
            app: app.clone(),
            delegate: delegate.clone(),
            main,
            image_view,
            rep: None,
            rep_size: (0, 0),
            frames: 0,
            fps_at: Instant::now(),
            fps: 0,
            dbg: None,
            ppu: None,
            settings: None,
        });
    });

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
        self.frames += 1;
        if self.fps_at.elapsed().as_secs() >= 1 {
            self.fps = self.frames as u32;
            self.frames = 0;
            self.fps_at = Instant::now();
            let tick = self.host.tick();
            let state = if !tick.loaded {
                "NO ROM"
            } else if tick.paused {
                "PAUSED"
            } else {
                "RUNNING"
            };
            self.main.setTitle(&NSString::from_str(&format!(
                "nerust — {} fps {state}",
                self.fps
            )));
        }
        self.present();
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

    fn open_debugger(&mut self) {
        if self.dbg.is_some() {
            return;
        }
        self.host
            .dispatch_cmd(nerust_gui_shell::session::commands::SessionCommand::Pause);
        self.dbg = super::dbg::DbgWin::open(self.mtm, &mut self.host, &self.delegate);
    }

    fn open_ppu(&mut self) {
        if self.ppu.is_some() {
            return;
        }
        self.ppu = super::ppu::PpuWin::open(self.mtm, &mut self.host);
    }

    fn open_settings(&mut self) {
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

fn build_menus(mtm: MainThreadMarker, app: &NSApplication, delegate: &AppDelegate) {
    let target: &AnyObject = anyobject(delegate);
    let bar = NSMenu::new(mtm);
    let submenu = |title: &str, items: &[(&str, Sel, &str)]| {
        let menu = NSMenu::initWithTitle(mtm.alloc::<NSMenu>(), &NSString::from_str(title));
        for (label, action, key) in items {
            let item: Retained<NSMenuItem> = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc::<NSMenuItem>(),
                    &NSString::from_str(label),
                    Some(*action),
                    &NSString::from_str(key),
                )
            };
            unsafe {
                item.setTarget(Some(target));
            }
            menu.addItem(&item);
        }
        let head: Retained<NSMenuItem> = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        head.setSubmenu(Some(&menu));
        bar.addItem(&head);
    };
    use objc2::runtime::Sel;
    submenu(
        "File",
        &[
            ("Open…", sel!(openFile:), "o"),
            ("Settings…", sel!(openSettings:), ","),
            ("Quit", sel!(quitApp:), "q"),
        ],
    );
    submenu(
        "Emulation",
        &[
            ("Pause", sel!(pauseEmulation:), ""),
            ("Resume", sel!(resumeEmulation:), ""),
            ("Reset", sel!(resetEmulation:), ""),
        ],
    );
    submenu(
        "State",
        &[
            ("New save slot", sel!(newSaveSlot:), ""),
            ("Save state", sel!(saveState:), ""),
            ("Load state", sel!(loadState:), ""),
            ("Next slot", sel!(nextSlot:), ""),
            ("Previous slot", sel!(prevSlot:), ""),
        ],
    );
    submenu(
        "View",
        &[
            ("Fullscreen", sel!(toggleFullscreen:), ""),
            ("Debugger", sel!(toggleDebugger:), ""),
            ("PPU viewer", sel!(togglePpu:), ""),
        ],
    );
    app.setMainMenu(Some(&bar));
}
