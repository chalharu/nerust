//! Main window: menu bar, game view, OSD overlay, frame timer.
//!
//! The window hosts a `MenuBar` (File/Emulation/State/Help, mirroring the
//! tao menu ids from `nerust_win::keys::menu`) above the game `Image`,
//! with a status `TextBlock` and toast `InfoBar` overlay. A 16ms
//! `DispatcherQueueTimer` drives the shared host turn; dynamic slot
//! submenus rebuild on the 1Hz chrome sync like the gtk twin.

use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_shell::{
    context::FrontendContext,
    registry::SystemRegistry,
    session::{access::SessionControl, commands::SessionCommand},
};
use nerust_win::{WinHost, keys::menu};
use windows_core::{Interface, Result};

use crate::bindings::Microsoft::UI::Windowing::AppWindowPresenterKind;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Grid, Image, InfoBar, InfoBarSeverity, MenuBar, MenuBarItem, MenuFlyoutItem, MenuFlyoutSubItem,
    Orientation, RowDefinition, StackPanel, TextBlock,
};
use crate::bindings::Microsoft::UI::Xaml::Media::Imaging::SoftwareBitmapSource;
use crate::bindings::Microsoft::UI::Xaml::Media::Stretch;
use crate::bindings::Microsoft::UI::Xaml::{
    Application, GridLengthHelper, RoutedEventHandler, Window,
};
use crate::bindings::Windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use crate::bindings::Windows::Storage::Pickers::FileOpenPicker;
use crate::bindings::Windows::Storage::Streams::DataWriter;
use crate::surface::WinUiSurface;

use super::{h, show_error_dialog};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.borrow_mut().as_mut().map(f))
}

pub struct App {
    pub host: WinHost,
    surface: WinUiSurface,
    window: Window,
    #[allow(dead_code)]
    frame_timer: crate::bindings::Windows::System::Threading::ThreadPoolTimer,
    hwnd: isize,
    last_focused: bool,
    game_source: SoftwareBitmapSource,
    status: TextBlock,
    toast: InfoBar,
    toast_seq: u64,
    debugger: Option<super::debugger::DebuggerWin>,
    ppu: Option<super::ppu::PpuWin>,
    settings: Option<super::settings::SettingsWin>,
    history: Option<super::history::HistoryWin>,
    slot_menus: Vec<SlotMenu>,
    keys_down: [bool; 256],
    text_focused: bool,
    frames: u64,
    fps_at: Instant,
    fps: u32,
}

/// Dynamic per-slot submenu (Select/Save/Load/Delete), rebuilt on chrome sync.
struct SlotMenu {
    kind: SlotMenuKind,
    submenu: MenuFlyoutSubItem,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotMenuKind {
    Select,
    Save,
    Load,
    Delete,
}

const HISTORY: u32 = 40040;
const ABOUT: u32 = 40041;

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetActiveWindow() -> *mut std::ffi::c_void;
    fn GetAsyncKeyState(vkey: i32) -> i16;
}

/// Whether our thread currently owns an active window (focus proxy: no
/// Window.Activated event in these bindings).
#[cfg(windows)]
fn active_window_focused() -> bool {
    unsafe { !GetActiveWindow().is_null() }
}

#[cfg(windows)]
pub(crate) fn active_window_raw() -> *mut std::ffi::c_void {
    unsafe { GetActiveWindow() }
}

#[cfg(not(windows))]
pub(crate) fn active_window_raw() -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

#[cfg(not(windows))]
fn active_window_focused() -> bool {
    true
}

/// polled gameplay keys (arrows/space/letters/digits/F-keys).
#[cfg(windows)]
fn poll_key(code: u16) -> bool {
    let down = unsafe { GetAsyncKeyState(code as i32) };
    down < 0
}

#[cfg(not(windows))]
fn poll_key(_code: u16) -> bool {
    false
}

const POLLED_KEYS: [u16; 57] = [
    0x25, 0x26, 0x27, 0x28, 0x20, 0x0D, 0x1B, 0x09, 0x08, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
    0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
    0x58, 0x59, 0x5A, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x70, 0x71, 0x72,
    0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B,
];

fn on_menu_id(id: u32) {
    with_app(|app| {
        if id == menu::FILE_OPEN {
            app.open_file_picker();
        } else if id == menu::FILE_CLOSE {
            SessionControl::unload(&mut app.host);
        } else if id == menu::FILE_EXIT {
            app.quit();
        } else if id == menu::PPU {
            app.open_ppu();
        } else if id == menu::SETTINGS {
            app.open_settings();
        } else if id == HISTORY {
            app.open_history();
        } else if id == ABOUT {
            app.open_about();
        } else {
            let mut surface = std::mem::take(&mut app.surface);
            app.host.on_menu(id, &mut surface);
            app.surface = surface;
        }
    });
}

fn menu_item(text: &str, hint: Option<&str>, id: u32) -> Result<MenuFlyoutItem> {
    let label = match hint {
        Some(shortcut) => format!("{text} ({shortcut})"),
        None => text.to_string(),
    };
    let item = MenuFlyoutItem::new()?;
    item.SetText(&h(&label))?;
    let handler_id = id;
    item.Click(&RoutedEventHandler::new(move |_, _| {
        on_menu_id(handler_id);
        Ok(())
    }))?;
    Ok(item)
}

fn submenu(title: &str) -> Result<MenuFlyoutSubItem> {
    let sub = MenuFlyoutSubItem::new()?;
    sub.SetText(&h(title))?;
    Ok(sub)
}

impl App {
    pub fn run_app() -> Result<()> {
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
        let host = WinHost::new(ctx).expect("session builds");

        let args: Vec<String> = std::env::args().collect();
        let rom = args.get(1).map(std::path::PathBuf::from);

        let app = App::open(host, rom)?;
        APP.with(|cell| {
            *cell.borrow_mut() = Some(app);
        });
        Ok(())
    }

    fn open(host: WinHost, rom: Option<std::path::PathBuf>) -> Result<Self> {
        let window = Window::new()?;
        window.SetTitle(&h("nerust"))?;
        window.SetExtendsContentIntoTitleBar(false)?;

        let bar = MenuBar::new()?;

        let game_source = SoftwareBitmapSource::new()?;
        let game = Image::new()?;
        game.SetSource(&game_source)?;
        game.SetStretch(Stretch::Uniform)?;

        let status = TextBlock::new()?;
        status.SetText(&h(""))?;

        let toast = InfoBar::new()?;
        toast.SetIsOpen(false)?;

        let overlay = StackPanel::new()?;
        overlay.SetOrientation(Orientation::Vertical)?;
        overlay.Children()?.Append(&status)?;
        overlay.Children()?.Append(&toast)?;

        let root = Grid::new()?;
        let rows = root.RowDefinitions()?;
        let menu_row = RowDefinition::new()?;
        menu_row.SetHeight(GridLengthHelper::Auto()?)?;
        let game_row = RowDefinition::new()?;
        game_row.SetHeight(crate::bindings::Microsoft::UI::Xaml::GridLength {
            Value: 1.0,
            GridUnitType: crate::bindings::Microsoft::UI::Xaml::GridUnitType::Star,
        })?;
        rows.Append(&menu_row)?;
        rows.Append(&game_row)?;
        root.Children()?.Append(&bar)?;
        Grid::SetRow(&bar, 0)?;
        root.Children()?.Append(&game)?;
        Grid::SetRow(&game, 1)?;

        window.SetContent(&root)?;
        window.Activate()?;

        let queue = window.DispatcherQueue()?;
        let timer_queue = queue.clone();
        let frame_timer =
            crate::bindings::Windows::System::Threading::ThreadPoolTimer::CreatePeriodicTimer(
                &crate::bindings::Windows::System::Threading::TimerElapsedHandler::new(move |_| {
                    let _ = timer_queue.TryEnqueue(
                        &crate::bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(
                            || {
                                with_app(|app| app.on_tick());
                                Ok(())
                            },
                        ),
                    );
                    Ok(())
                }),
                crate::bindings::Windows::Foundation::TimeSpan { Duration: 160_000 },
            )?;

        // Focus tracking bubbles from any control: text inputs set the
        // typing guard so polled gameplay keys don't leak into fields.
        root.GotFocus(
            &crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler::new(|sender, _| {
                let text = sender.as_ref().is_some_and(|sender| {
                    sender
                        .cast::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>()
                        .is_ok()
                        || sender
                            .cast::<crate::bindings::Microsoft::UI::Xaml::Controls::ComboBox>()
                            .is_ok()
                });
                with_app(|app| {
                    app.text_focused = text;
                });
                Ok(())
            }),
        )?;
        root.LostFocus(
            &crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler::new(|_, _| {
                with_app(|app| {
                    app.text_focused = false;
                });
                Ok(())
            }),
        )?;
        let hwnd = super::window_handle(&window)?;

        let mut app = Self {
            host,
            surface: WinUiSurface::default(),
            window,
            frame_timer,
            hwnd,
            last_focused: true,
            game_source,
            status,
            toast,
            toast_seq: 0,
            debugger: None,
            ppu: None,
            settings: None,
            history: None,
            slot_menus: Vec::new(),
            keys_down: [false; 256],
            text_focused: false,
            frames: 0,
            fps_at: Instant::now(),
            fps: 0,
        };
        app.build_menus(&bar)?;
        if let Some(path) = rom
            && let Err(detail) = app.host.load_path(&path)
        {
            let root = app.xaml_root();
            show_error_dialog(&root, "ROM load failed", &detail);
        }
        Ok(app)
    }

    fn xaml_root(&self) -> crate::bindings::Microsoft::UI::Xaml::XamlRoot {
        self.window
            .Content()
            .expect("content")
            .XamlRoot()
            .expect("xaml root")
    }

    fn build_menus(&mut self, bar: &MenuBar) -> Result<()> {
        let file = MenuBarItem::new()?;
        file.SetTitle(&h("File"))?;
        let items = file.Items()?;
        items.Append(&menu_item("Open ROM...", Some("Ctrl+O"), menu::FILE_OPEN)?)?;
        items.Append(&menu_item("Close", None, menu::FILE_CLOSE)?)?;
        items.Append(&menu_item("Settings...", None, menu::SETTINGS)?)?;
        items.Append(&menu_item("Quit", None, menu::FILE_EXIT)?)?;
        bar.Items()?.Append(&file)?;

        let emulation = MenuBarItem::new()?;
        emulation.SetTitle(&h("Emulation"))?;
        let items = emulation.Items()?;
        items.Append(&menu_item("Pause", None, menu::PAUSE)?)?;
        items.Append(&menu_item("Resume", None, menu::RESUME)?)?;
        items.Append(&menu_item("Reset", None, menu::RESET)?)?;
        let state = submenu("Save States")?;
        let state_items = state.Items()?;
        state_items.Append(&menu_item("Create New Slot", None, menu::CREATE_SLOT)?)?;
        state_items.Append(&menu_item("Save Active Slot", Some("F5"), menu::SAVE_SLOT)?)?;
        state_items.Append(&menu_item("Load Active Slot", Some("F8"), menu::LOAD_SLOT)?)?;
        state_items.Append(&menu_item("Next Slot", Some("F6"), menu::NEXT_SLOT)?)?;
        state_items.Append(&menu_item("Previous Slot", Some("F7"), menu::PREV_SLOT)?)?;
        for (title, kind) in [
            ("Select Active Slot", SlotMenuKind::Select),
            ("Save Slot", SlotMenuKind::Save),
            ("Load Slot", SlotMenuKind::Load),
            ("Delete Slot", SlotMenuKind::Delete),
        ] {
            let sub = submenu(title)?;
            state_items.Append(&sub)?;
            self.slot_menus.push(SlotMenu { kind, submenu: sub });
        }
        items.Append(&state)?;
        items.Append(&menu_item("Open Debugger", Some("F4"), menu::DEBUGGER)?)?;
        items.Append(&menu_item("Notification History", None, HISTORY)?)?;
        bar.Items()?.Append(&emulation)?;

        let help = MenuBarItem::new()?;
        help.SetTitle(&h("Help"))?;
        let items = help.Items()?;
        items.Append(&menu_item("About", None, ABOUT)?)?;
        bar.Items()?.Append(&help)?;
        Ok(())
    }

    fn on_tick(&mut self) {
        // No Closed/Activated events in these bindings: poll visibility
        // for quit and the foreground window for focus.
        match self.window.Visible() {
            Ok(false) => {
                let _ = Application::Current().and_then(|app| app.Exit());
                return;
            }
            Err(error) => {
                log::warn!("winui visible poll failed: {error}");
            }
            Ok(true) => {}
        }
        let focused = active_window_focused();
        if focused != self.last_focused {
            self.last_focused = focused;
            if !focused {
                self.release_all_keys();
            }
            self.host.on_focus(focused);
        }
        if focused {
            self.poll_keys();
        }
        self.host.turn();
        self.host.swap_frame_buffer();
        if let Some(dbg) = self.debugger.as_mut() {
            dbg.poll(&mut self.host);
        }
        if let Some(ppu) = self.ppu.as_mut() {
            ppu.poll(&mut self.host);
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
        if self.status.SetText(&h(&line)).is_err() {
            log::warn!("winui status update failed");
        }
        let title = self
            .host
            .session_mut()
            .window_title_cached(&tick)
            .to_string();
        if self.window.SetTitle(&h(&title)).is_err() {
            log::warn!("winui title update failed");
        }
        self.rebuild_slot_menus();
        self.poll_toast();
        if self.history.is_some() {
            self.refresh_history();
        }
    }

    fn rebuild_slot_menus(&mut self) {
        // Dynamic per-slot submenus (Select/Save/Load/Delete), mirroring
        // the gtk/tao twins. Rebuilt on the 1Hz chrome sync.
        if self.slot_menus.is_empty() {
            return;
        }
        let session = self.host.session();
        let slots = session.slots().to_vec();
        let active = session.active_slot_id();
        let tick = self.host.tick();
        let loaded = tick.loaded;
        for entry in std::mem::take(&mut self.slot_menus) {
            let items = match entry.submenu.Items() {
                Ok(items) => items,
                Err(_) => {
                    self.slot_menus.push(entry);
                    continue;
                }
            };
            let _ = items.Clear();
            if slots.is_empty() {
                if let Ok(empty) = menu_item("(No save slots)", None, 0) {
                    let _ = empty.SetIsEnabled(false);
                    let _ = items.Append(&empty);
                }
            } else {
                for slot in &slots {
                    let label = slot_label(slot, active);
                    let (kind, id) = (entry.kind, slot.slot_id);
                    let item = menu_item(&label, None, 0);
                    if let Ok(item) = item {
                        let _ = item.Click(&RoutedEventHandler::new(move |_, _| {
                            with_app(|app| {
                                app.host.dispatch_cmd(match kind {
                                    SlotMenuKind::Select => SessionCommand::SelectActiveSlot(id),
                                    SlotMenuKind::Save => SessionCommand::SaveSlot(id),
                                    SlotMenuKind::Load => SessionCommand::LoadSlot(id),
                                    SlotMenuKind::Delete => SessionCommand::DeleteSlot(id),
                                });
                            });
                            Ok(())
                        }));
                        let _ = item.SetIsEnabled(loaded);
                        let _ = items.Append(&item);
                    }
                }
            }
            self.slot_menus.push(entry);
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
            use nerust_gui_shell::session::notification::NotificationLevel;
            let severity = match last.level {
                NotificationLevel::Info => InfoBarSeverity::Informational,
                NotificationLevel::Warning => InfoBarSeverity::Warning,
                NotificationLevel::Error => InfoBarSeverity::Error,
            };
            let _ = self.toast.SetSeverity(severity);
            let _ = self.toast.SetTitle(&h(last.module));
            let _ = self.toast.SetMessage(&h(&last.text));
            let _ = self.toast.SetIsOpen(true);
        } else if self.toast.SetIsOpen(false).is_err() {
            log::warn!("winui toast update failed");
        }
    }

    /// Frame-rate keyboard poll (no KeyDown events in these bindings).
    /// Gameplay keys only; typing is guarded by the focus tracker, and
    /// shortcuts stay on the native menu/accelerator path.
    /// Frame-rate gameplay-key poll (no KeyDown events on Window in
    /// these bindings). Routing mirrors the gtk twins: settings capture
    /// wins, then debugger function keys, then shared shortcuts; typing
    /// in text fields is guarded by the focus tracker.
    fn poll_keys(&mut self) {
        use nerust_frontend_host::debugger::DebuggerEvent as E;
        let mut events = Vec::new();
        for code in POLLED_KEYS {
            let down = poll_key(code);
            let index = code as usize;
            if index < self.keys_down.len() && down != self.keys_down[index] {
                self.keys_down[index] = down;
                events.push((code, down));
            }
        }
        if self.is_settings_capturing() {
            for (code, down) in events {
                if down && let Some(key) = nerust_win::keys::win_key_to_nerust(code) {
                    self.capture_key(key);
                }
            }
            return;
        }
        if self.text_focused {
            return;
        }
        let main_active = unsafe { GetActiveWindow() } as isize == self.hwnd;
        for (code, down) in events {
            if main_active {
                self.host.on_key(code, down);
            } else if down && self.debugger.is_some() {
                let event = match code {
                    0x75 => Some(E::StepFramePressed),
                    0x76 => Some(E::StepInstrPressed),
                    0x78 => Some(E::TogglePausePressed),
                    _ => None,
                };
                if let Some(event) = event {
                    self.debug_gesture(event);
                }
            }
        }
    }

    fn release_all_keys(&mut self) {
        for (code, down) in self.keys_down.iter_mut().enumerate() {
            if *down {
                *down = false;
                self.host.on_key(code as u16, false);
            }
        }
    }

    fn present(&mut self) {
        let Some((w, h, bgrx)) = self.host_framebuffer() else {
            return;
        };
        let bitmap = (|| -> Result<()> {
            let writer = DataWriter::new()?;
            writer.WriteBytes(&bgrx)?;
            let buffer = writer.DetachBuffer()?;
            let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
                &buffer,
                BitmapPixelFormat::Bgra8,
                w as i32,
                h as i32,
            )?;
            self.game_source.SetBitmapAsync(&bitmap)?;
            Ok(())
        })();
        if let Err(error) = bitmap {
            log::warn!("winui present failed: {error}");
        }
    }

    /// Owned copy of the current frame as BGRX bytes. Cores emit palette
    /// indices; the palette table rides the frame.
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
                out[o + 3] = 0xFF;
            }
        }
        Some((w, h, out))
    }

    fn drain_surface(&mut self) {
        let surface = std::mem::take(&mut self.surface);
        for title in &surface.titles {
            if self.window.SetTitle(&h(title)).is_err() {
                log::warn!("winui title effect failed");
            }
        }
        for &fullscreen in &surface.fullscreen_calls {
            if let Err(error) = self.apply_fullscreen(fullscreen) {
                log::warn!("winui fullscreen failed: {error}");
            }
        }
        for detail in &surface.modal_errors {
            show_error_dialog(&self.xaml_root(), "nerust", detail);
        }
        if surface.debugger_requests > 0 {
            self.open_debugger();
        }
        self.surface = surface;
    }

    fn apply_fullscreen(&mut self, fullscreen: bool) -> Result<()> {
        let window = self.window.AppWindow()?;
        window.SetPresenterByKind(if fullscreen {
            AppWindowPresenterKind::FullScreen
        } else {
            AppWindowPresenterKind::Default
        })?;
        Ok(())
    }

    fn quit(&mut self) {
        if let Err(error) = Application::Current().and_then(|app| app.Exit()) {
            log::error!("winui quit failed: {error}");
        }
    }

    pub(crate) fn open_debugger(&mut self) {
        if self.debugger.is_some() {
            return;
        }
        self.host.dispatch_cmd(SessionCommand::Pause);
        self.debugger = super::debugger::DebuggerWin::open(&mut self.host);
    }

    pub fn close_debugger(&mut self) {
        self.debugger = None;
    }

    pub fn close_ppu(&mut self) {
        self.ppu = None;
    }

    pub fn close_settings_win(&mut self) {
        self.settings = None;
    }

    pub fn close_history(&mut self) {
        self.history = None;
    }

    pub fn debug_gesture(&mut self, event: nerust_frontend_host::debugger::DebuggerEvent) {
        let (actions, open_ppu) = match self.debugger.as_mut() {
            Some(dbg) => {
                dbg.echo_inputs();
                let update = dbg.model.dispatch(event);
                (update.actions, update.open_ppu)
            }
            None => return,
        };
        if open_ppu {
            self.open_ppu();
        }
        let (debugger, host) = (&mut self.debugger, &mut self.host);
        if let Some(dbg) = debugger {
            dbg.act(host, &actions);
        }
    }

    pub fn debug_follow(&mut self, index: i32) {
        let target = self
            .debugger
            .as_ref()
            .and_then(|dbg| dbg.model.display.disasm_lines.get(index as usize))
            .and_then(|line| line.target);
        if let Some(addr) = target {
            self.debug_gesture(nerust_frontend_host::debugger::DebuggerEvent::FollowTarget(
                addr,
            ));
        }
    }

    pub fn debug_follow_selected(&mut self) {
        let target = self
            .debugger
            .as_ref()
            .and_then(|dbg| dbg.selected_dis_row())
            .and_then(|index| {
                self.debugger
                    .as_ref()
                    .and_then(|dbg| dbg.model.display.disasm_lines.get(index))
                    .and_then(|line| line.target)
            });
        if let Some(addr) = target {
            self.debug_gesture(nerust_frontend_host::debugger::DebuggerEvent::FollowTarget(
                addr,
            ));
        }
    }

    pub fn debug_select_dis_row(&mut self, index: i32) {
        let addr = self
            .debugger
            .as_ref()
            .and_then(|dbg| dbg.model.display.disasm_lines.get(index as usize))
            .map(|line| line.addr);
        if let Some(addr) = addr {
            self.debug_gesture(nerust_frontend_host::debugger::DebuggerEvent::DisLineClicked(addr));
        }
    }

    pub fn debug_select_row(&mut self, index: i32) {
        let addr = self
            .debugger
            .as_ref()
            .and_then(|dbg| dbg.model.display.dump_rows.get(index as usize))
            .map(|row| row.addr);
        if let Some(addr) = addr {
            self.debug_gesture(nerust_frontend_host::debugger::DebuggerEvent::RowSelected(
                addr,
            ));
        }
    }

    pub fn update_settings(&mut self, event: nerust_frontend_host::settings::SettingsEvent) {
        if let Some(dlg) = self.settings.as_mut() {
            let _ = dlg.controller_mut().update(event);
            if dlg.rebuild_page().is_err() {
                log::warn!("winui settings rebuild failed");
            }
        }
    }

    pub fn update_settings_no_rebuild(
        &mut self,
        event: nerust_frontend_host::settings::SettingsEvent,
    ) {
        if let Some(dlg) = self.settings.as_mut() {
            let _ = dlg.controller_mut().update(event);
            dlg.refresh_chrome();
        }
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

    pub fn select_settings_page(&mut self, index: usize) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.set_page(index);
            if dlg.rebuild_page().is_err() {
                log::warn!("winui settings page rebuild failed");
            }
        }
    }

    pub fn is_settings_capturing(&self) -> bool {
        self.settings.as_ref().is_some_and(|dlg| dlg.is_capturing())
    }

    pub fn capture_key(&mut self, key: nerust_keyboard::Key) {
        if self.is_settings_capturing() {
            self.update_settings(nerust_frontend_host::settings::SettingsEvent::CaptureKey(
                key,
            ));
        }
    }

    pub fn browse_storage_directory(&mut self) {
        if let Some(dlg) = self.settings.as_mut() {
            dlg.browse_storage_directory();
        }
    }

    pub fn refresh_history(&mut self) {
        if let (Some(win), host) = (&mut self.history, &mut self.host) {
            win.refresh(host);
        }
    }

    pub fn clear_history(&mut self) {
        self.host.clear_history();
        self.refresh_history();
    }

    pub(crate) fn open_ppu(&mut self) {
        if self.ppu.is_some() {
            return;
        }
        self.ppu = super::ppu::PpuWin::open(&mut self.host);
    }

    pub(crate) fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }
        self.settings = super::settings::SettingsWin::open(&mut self.host);
    }

    pub(crate) fn open_history(&mut self) {
        if self.history.is_some() {
            return;
        }
        self.history = super::history::HistoryWin::open(&mut self.host);
    }

    fn open_about(&mut self) {
        show_error_dialog(
            &self.xaml_root(),
            "About nerust",
            "nerust WinUI frontend prototype",
        );
    }

    fn open_file_picker(&mut self) {
        let picker = match FileOpenPicker::new() {
            Ok(picker) => picker,
            Err(error) => {
                log::error!("winui picker failed: {error}");
                return;
            }
        };
        let flow = (|| -> Result<()> {
            use crate::bindings::Windows::Win32::Foundation::HWND;
            use crate::bindings::Windows::Win32::UI::Shell::IInitializeWithWindow;
            let hwnd = super::window_handle(&self.window)?;
            let init: IInitializeWithWindow = picker.cast()?;
            unsafe { init.Initialize(HWND(hwnd as *mut std::ffi::c_void))? };
            picker.FileTypeFilter()?.Append(&h(".nes"))?;
            let op = picker.PickSingleFileAsync()?;
            let queue = self.window.DispatcherQueue()?;
            op.SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(
                    move |op, _| {
                        use crate::bindings::Windows::Storage::StorageFile;
                        use windows_future::IAsyncOperation;
                        let path = op
                            .as_ref()
                            .and_then(|op: &IAsyncOperation<StorageFile>| op.GetResults().ok())
                            .and_then(|file: StorageFile| file.Path().ok())
                            .map(|path| std::path::PathBuf::from(path.to_string()));
                        if let Some(path) = path {
                            let queue = queue.clone();
                            queue.TryEnqueue(
                                &crate::bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(
                                    move || {
                                        with_app(|app| {
                                            if let Err(detail) = app.host.load_path(&path) {
                                                let root = app.xaml_root();
                                                show_error_dialog(
                                                    &root,
                                                    "ROM load failed",
                                                    &detail,
                                                );
                                            }
                                        });
                                        Ok(())
                                    },
                                ),
                            )?;
                        }
                        Ok(())
                    },
                ),
            )?;
            Ok(())
        })();
        if let Err(error) = flow {
            log::error!("winui file picker failed: {error}");
        }
    }
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
            "winui demo renders via SoftwareBitmapSource",
            Box::new(std::io::Error::other("no GPU pipeline")),
        ))
    }
}
