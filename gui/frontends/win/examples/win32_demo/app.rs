//! Demo application: one main window, one message loop, GDI presentation.
//!
//! Emulation runs on the shared session thread; this file only pumps
//! Windows messages, forwards them to [`nerust_win::WinHost`], and blits
//! the latest core framebuffer with GDI. OSD is a `TextOutW` status line
//! drawn over the blit. Debugger, PPU viewer, and settings are separate
//! `HWND`s in `dbg.rs`, `ppu.rs`, and `settings.rs`; this file owns their
//! lifetime slots plus the shared `with_*` split-borrow accessors.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use super::fluent;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_shell::{context::FrontendContext, registry::SystemRegistry};
use nerust_win::{WinHost, WinSurface, keys::menu};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::*,
        UI::Controls::Dialogs::{
            GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
        UI::WindowsAndMessaging::*,
    },
    core::*,
};

pub const WM_APP_OPEN_DEBUGGER: u32 = WM_APP + 1;
pub const WM_APP_SET_FULLSCREEN: u32 = WM_APP + 2;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// RGBA core/live bytes to top-down BGRX DIB bytes.
pub fn rgba_to_bgrx(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        let s = i * 4;
        if s + 3 >= rgba.len() {
            break;
        }
        out[s] = rgba[s + 2];
        out[s + 1] = rgba[s + 1];
        out[s + 2] = rgba[s];
        out[s + 3] = 0;
    }
    out
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                eprintln!("win32: reentrant APP borrow dropped");
                return None;
            }
        };
        borrow.as_mut().map(f)
    })
}

/// Split borrow: debugger window plus the shared host.
pub fn with_dbg<R>(f: impl FnOnce(&mut super::dbg::DbgWin, &mut WinHost) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                eprintln!("win32: reentrant APP borrow dropped");
                return None;
            }
        };
        let app = match borrow.as_mut() {
            Some(app) => app,
            None => return None,
        };
        let (dbg, host) = (&mut app.dbg, &mut app.host);
        dbg.as_mut().map(|dbg| f(dbg, host))
    })
}

pub fn close_dbg(hwnd: HWND) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut()
            && let Some(dbg) = app.dbg.as_ref()
            && dbg.hwnd == hwnd
        {
            app.dbg = None;
        }
    });
}

/// Split borrow: PPU window plus the shared host.
pub fn with_ppu<R>(f: impl FnOnce(&mut super::ppu::PpuWin, &mut WinHost) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                eprintln!("win32: reentrant APP borrow dropped");
                return None;
            }
        };
        let app = match borrow.as_mut() {
            Some(app) => app,
            None => return None,
        };
        let (ppu, host) = (&mut app.ppu, &mut app.host);
        ppu.as_mut().map(|ppu| f(ppu, host))
    })
}

pub fn close_ppu(hwnd: HWND) {
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut()
            && let Some(ppu) = app.ppu.as_ref()
            && ppu.hwnd == hwnd
        {
            app.ppu = None;
        }
    });
}

/// Split borrow: settings dialog plus the shared host.
pub fn with_settings<R>(
    f: impl FnOnce(&mut super::settings::SettingsDlg, &mut WinHost) -> R,
) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                eprintln!("win32: reentrant APP borrow dropped");
                return None;
            }
        };
        let app = match borrow.as_mut() {
            Some(app) => app,
            None => return None,
        };
        let (settings, host) = (&mut app.settings, &mut app.host);
        settings.as_mut().map(|settings| f(settings, host))
    })
}

pub fn close_settings(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    APP.with(|cell| {
        if let Some(app) = cell.borrow_mut().as_mut()
            && let Some(settings) = app.settings.as_ref()
            && settings.hwnd == hwnd
        {
            unsafe {
                let _ = EnableWindow(settings.parent, true);
            }
            app.settings = None;
        }
    });
}

/// Native effect sink: chrome effects become posted messages so the
/// `WndProc` borrow never reenters the app borrow.
pub struct EffectSurface {
    pub hwnd: HWND,
}

impl WinSurface for EffectSurface {
    fn set_title(&mut self, title: &str) {
        let wide = wide(title);
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        unsafe {
            let _ = PostMessageW(
                self.hwnd,
                WM_APP_SET_FULLSCREEN,
                WPARAM(fullscreen as usize),
                LPARAM(0),
            );
        }
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        None
    }

    fn show_error_modal(&mut self, detail: &str) {
        let wide = wide(detail);
        unsafe {
            MessageBoxW(
                self.hwnd,
                PCWSTR(wide.as_ptr()),
                w!("nerust"),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    fn request_debugger(&mut self) {
        unsafe {
            let _ = PostMessageW(self.hwnd, WM_APP_OPEN_DEBUGGER, WPARAM(0), LPARAM(0));
        }
    }
}

/// `extern "system"` aborts on unwind: convert handler panics into log
/// lines so one bad paint never kills the process.
fn guard<R: Default>(what: &str, f: impl FnOnce() -> R + std::panic::UnwindSafe) -> R {
    match std::panic::catch_unwind(f) {
        Ok(value) => value,
        Err(_) => {
            log::error!("win32 handler panicked: {what}");
            R::default()
        }
    }
}

pub fn run() {
    let rom: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            unsafe {
                MessageBoxW(
                    HWND::default(),
                    w!("usage: win32_demo <rom path>"),
                    w!("nerust win32 demo"),
                    MB_OK | MB_ICONERROR,
                );
            }
            std::process::exit(2);
        });

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
    let mut host = WinHost::new(ctx).expect("session builds");
    if let Err(detail) = host.load_path(&rom) {
        unsafe {
            let message = HSTRING::from(format!("ROM load failed: {detail}"));
            MessageBoxW(
                HWND::default(),
                PCWSTR(message.as_ptr()),
                w!("nerust win32 demo"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }

    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
        let class = w!("NerustWin32Demo");
        let wcex = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: class,
            ..Default::default()
        };
        assert!(RegisterClassExW(&wcex) != 0);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("nerust win32 demo"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            768,
            720,
            HWND(0),
            HMENU(0),
            instance,
            None,
        );
        assert!(hwnd != HWND::default(), "main window");

        build_menu(hwnd);
        fluent::apply_chrome(hwnd);
        SetTimer(hwnd, TIMER_FRAME, 16, None);

        APP.with(|cell| {
            *cell.borrow_mut() = Some(App::new(host, hwnd));
        });

        // Verification hooks: open aux windows without X input, which the
        // headless Xvfb session cannot reliably deliver to Wine menus.
        // drive_submit runs outside the APP borrow: it borrows APP itself
        // through with_settings.
        let args: Vec<String> = std::env::args().collect();
        with_app(|app| {
            if args.iter().any(|a| a == "--pause") {
                app.host
                    .dispatch_cmd(nerust_gui_shell::session::commands::SessionCommand::Pause);
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
            with_app(|app| app.set_fullscreen(true));
        }
        // Modal common dialog: blocks the loop by design. Screenshot it,
        // then kill the process; path plumbing shares host.load_path.
        if args.iter().any(|a| a == "--file-open") {
            with_app(|app| app.open_rom_file());
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

const TIMER_FRAME: usize = 1;

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
            "test",
            Box::new(std::io::Error::other("GDI demo needs no GPU pipeline")),
        ))
    }
}

struct App {
    host: WinHost,
    hwnd: HWND,
    frames: u64,
    fps_at: Instant,
    fps: u32,
    dbg: Option<super::dbg::DbgWin>,
    ppu: Option<super::ppu::PpuWin>,
    settings: Option<super::settings::SettingsDlg>,
    fs_prev: Option<(isize, RECT)>,
}

impl App {
    fn new(host: WinHost, hwnd: HWND) -> Self {
        Self {
            host,
            hwnd,
            frames: 0,
            fps_at: Instant::now(),
            fps: 0,
            dbg: None,
            ppu: None,
            settings: None,
            fs_prev: None,
        }
    }

    fn on_timer(&mut self) {
        self.host.turn();
        self.host.swap_frame_buffer();
        self.frames += 1;
        if self.fps_at.elapsed().as_secs() >= 1 {
            self.fps = self.frames as u32;
            self.frames = 0;
            self.fps_at = Instant::now();
        }
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, FALSE);
        }
    }

    fn on_paint(&mut self) {
        unsafe {
            let ps = &mut PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, ps);
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rect);
            let (cw, ch) = (
                (rect.right - rect.left).max(1),
                (rect.bottom - rect.top).max(1),
            );
            if let Some((w, h, bgrx)) = self.host_framebuffer() {
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w as i32;
                bmi.bmiHeader.biHeight = -(h as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
                StretchDIBits(
                    hdc,
                    0,
                    0,
                    cw,
                    ch,
                    0,
                    0,
                    w as i32,
                    h as i32,
                    Some(bgrx.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
            self.paint_osd(hdc);
            let _ = EndPaint(self.hwnd, ps);
        }
    }

    /// Owned copy of the current frame as BGRX bytes, width, height.
    /// Cores emit palette indices; the palette table rides the frame.
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

    fn paint_osd(&mut self, hdc: HDC) {
        unsafe {
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
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(fluent::FLUENT_TEXT));
            // Fluent body type for the status line; restored after drawing.
            let font = fluent::ui_font();
            let prev = SelectObject(hdc, font);
            let wide = wide(&line);
            let _ = TextOutW(hdc, 8, 8, &wide[..wide.len() - 1]);
            let _ = SelectObject(hdc, prev);
        }
    }

    fn open_debugger(&mut self) {
        if let Some(dbg) = self.dbg.as_ref()
            && unsafe { IsWindow(dbg.hwnd).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(dbg.hwnd);
            }
            return;
        }
        // Inspect requires a paused core: opening the debugger pauses,
        // like a hardware halt. Resume explicitly from either window.
        self.host
            .dispatch_cmd(nerust_gui_shell::session::commands::SessionCommand::Pause);
        self.dbg = super::dbg::DbgWin::open(&mut self.host);
    }

    fn open_ppu(&mut self) {
        if let Some(ppu) = self.ppu.as_ref()
            && unsafe { IsWindow(ppu.hwnd).as_bool() }
        {
            unsafe {
                let _ = SetForegroundWindow(ppu.hwnd);
            }
            return;
        }
        self.ppu = super::ppu::PpuWin::open();
    }

    fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }
        self.settings = super::settings::SettingsDlg::open(self.hwnd, &mut self.host);
    }

    fn open_rom_file(&mut self) {
        let path = unsafe {
            let mut file = vec![0u16; 512];
            let filter: Vec<u16> = "NES ROMs (*.nes)\0*.nes\0All Files (*.*)\0*.*\0\0"
                .encode_utf16()
                .collect();
            let mut ofn = OPENFILENAMEW::default();
            ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
            ofn.hwndOwner = self.hwnd;
            ofn.lpstrFilter = PCWSTR(filter.as_ptr());
            ofn.lpstrFile = PWSTR(file.as_mut_ptr());
            ofn.nMaxFile = file.len() as u32;
            ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
            if !GetOpenFileNameW(&mut ofn).as_bool() {
                return;
            }
            let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
            PathBuf::from(String::from_utf16_lossy(&file[..len]))
        };
        if let Err(detail) = self.host.load_path(&path) {
            let mut surface = EffectSurface { hwnd: self.hwnd };
            surface.show_error_modal(&detail);
        }
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        unsafe {
            if fullscreen && self.fs_prev.is_none() {
                let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                let mut rect = RECT::default();
                let _ = GetWindowRect(self.hwnd, &mut rect);
                self.fs_prev = Some((style, rect));
                SetWindowLongPtrW(
                    self.hwnd,
                    GWL_STYLE,
                    (style as u32 & !(WS_CAPTION.0 | WS_THICKFRAME.0)) as isize,
                );
                let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    let r = info.rcMonitor;
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_FRAMECHANGED,
                    );
                }
            } else if !fullscreen && let Some((style, rect)) = self.fs_prev.take() {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, style);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_FRAMECHANGED,
                );
            }
        }
    }
}

fn build_menu(hwnd: HWND) {
    unsafe {
        let bar = CreateMenu().unwrap();
        let file = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(
            file,
            MF_STRING,
            menu::FILE_OPEN as usize,
            w!("&Open...\tCtrl+O"),
        );
        let _ = AppendMenuW(file, MF_STRING, menu::FILE_CLOSE as usize, w!("&Close"));
        let _ = AppendMenuW(file, MF_STRING, menu::FILE_EXIT as usize, w!("E&xit"));
        let _ = AppendMenuW(bar, MF_POPUP, file.0 as usize, w!("&File"));

        let emulation = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(emulation, MF_STRING, menu::PAUSE as usize, w!("&Pause"));
        let _ = AppendMenuW(emulation, MF_STRING, menu::RESUME as usize, w!("&Resume"));
        let _ = AppendMenuW(emulation, MF_STRING, menu::RESET as usize, w!("&Reset"));
        let _ = AppendMenuW(bar, MF_POPUP, emulation.0 as usize, w!("&Emulation"));

        let state = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(
            state,
            MF_STRING,
            menu::CREATE_SLOT as usize,
            w!("&New save slot"),
        );
        let _ = AppendMenuW(
            state,
            MF_STRING,
            menu::SAVE_SLOT as usize,
            w!("&Save state"),
        );
        let _ = AppendMenuW(
            state,
            MF_STRING,
            menu::LOAD_SLOT as usize,
            w!("&Load state"),
        );
        let _ = AppendMenuW(state, MF_STRING, menu::NEXT_SLOT as usize, w!("Next slot"));
        let _ = AppendMenuW(
            state,
            MF_STRING,
            menu::PREV_SLOT as usize,
            w!("Previous slot"),
        );
        let _ = AppendMenuW(bar, MF_POPUP, state.0 as usize, w!("&State"));

        let view = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(
            view,
            MF_STRING,
            menu::FULLSCREEN as usize,
            w!("&Fullscreen"),
        );
        let _ = AppendMenuW(view, MF_STRING, menu::DEBUGGER as usize, w!("&Debugger"));
        let _ = AppendMenuW(view, MF_STRING, menu::PPU as usize, w!("&PPU viewer"));
        let _ = AppendMenuW(view, MF_STRING, menu::SETTINGS as usize, w!("&Settings..."));
        let _ = AppendMenuW(bar, MF_POPUP, view.0 as usize, w!("&View"));

        let _ = SetMenu(hwnd, bar);
        let _ = DrawMenuBar(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            guard("timer", || {
                with_app(|app| {
                    if hwnd == app.hwnd {
                        app.on_timer();
                    }
                });
            });
            LRESULT(0)
        }
        WM_PAINT => {
            guard("paint", || {
                with_app(|app| {
                    if hwnd == app.hwnd {
                        app.on_paint();
                    }
                });
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            with_app(|app| {
                app.host.on_key(vk, true);
            });
            LRESULT(0)
        }
        WM_KEYUP => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            with_app(|app| {
                app.host.on_key(vk, false);
            });
            LRESULT(0)
        }
        WM_SETFOCUS => {
            with_app(|app| app.host.on_focus(true));
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            with_app(|app| app.host.on_focus(false));
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u32;
            // DestroyWindow pumps WM_DESTROY synchronously: never run it
            // while the APP borrow is held (the nested proc would collide).
            // Window creation may drop one focus event under Wine; the
            // debugger open pauses explicitly, so nothing is lost.
            let exit = with_app(|app| {
                let mut surface = EffectSurface { hwnd: app.hwnd };
                // Shell-level ids handled around the host map.
                if id == menu::FILE_EXIT {
                    true
                } else if id == menu::FILE_CLOSE {
                    nerust_gui_shell::session::access::SessionControl::unload(&mut app.host);
                    false
                } else if id == menu::FILE_OPEN {
                    app.open_rom_file();
                    false
                } else if id == menu::PPU {
                    app.open_ppu();
                    false
                } else if id == menu::SETTINGS {
                    app.open_settings();
                    false
                } else {
                    app.host.on_menu(id, &mut surface);
                    false
                }
            })
            .unwrap_or(false);
            if exit {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        m if m == WM_APP_OPEN_DEBUGGER => {
            with_app(|app| app.open_debugger());
            LRESULT(0)
        }
        m if m == WM_APP_SET_FULLSCREEN => {
            let fullscreen = wparam.0 != 0;
            with_app(|app| app.set_fullscreen(fullscreen));
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
