//! Native loop adapted from prototype_1; rendering and futures remain UI-local.
use super::{Message, View, input, panic_error, scheduler::Scheduler};
use iced::{Size, mouse};
#[cfg(not(target_os = "linux"))]
use iced_tiny_skia::{
    graphics::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    graphics::Viewport,
    runtime::user_interface::{Cache, State, UserInterface},
};
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{Arc, Mutex, Once},
    time::Instant,
};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::run_return::EventLoopExtRunReturn,
    window::{Window, WindowBuilder},
};
use wakui2_runtime::Executor;
#[cfg(target_os = "linux")]
use {gtk::prelude::*, tao::platform::unix::WindowExtUnix};

#[derive(Debug)]
enum Native {
    Wake(usize),
    Refresh,
    Turn,
    Menu(muda::MenuId),
}
#[derive(Clone)]
struct Proxy(EventLoopProxy<Native>);
#[cfg(target_os = "linux")]
static NUDGE_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
impl Proxy {
    fn send(&self, event: Native) {
        if self.0.send_event(event).is_err() {
            return;
        }
        // Tao Linux's crossbeam proxy does not wake blocking gtk_main_iteration_do.
        // This is a one-shot wake, not a periodic poll or a cross-thread UI call.
        #[cfg(target_os = "linux")]
        if !NUDGE_PENDING.swap(true, std::sync::atomic::Ordering::AcqRel) {
            gtk::glib::idle_add_once(|| {
                NUDGE_PENDING.store(false, std::sync::atomic::Ordering::Release);
            });
        }
    }
}
// muda's process-global OnceLock handler cannot be cleared with None.
static MENU_PROXY: Mutex<Option<Proxy>> = Mutex::new(None);
static MENU_INIT: Once = Once::new();
struct MenuBridge;
impl Drop for MenuBridge {
    fn drop(&mut self) {
        MENU_PROXY.lock().unwrap().take();
    }
}
struct Session {
    view: View,
    executor: Rc<Scheduler>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.view.close();
        self.executor.close();
    }
}
struct ExecutorGuard(Rc<Scheduler>);
impl Drop for ExecutorGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

// iced_winit::Clipboard::connect requires Arc<winit::Window>, not a Tao window.
// Use its identical lower layer, retaining the native display owner until drop.
struct Clipboard {
    native: Option<window_clipboard::Clipboard>,
    _window: Arc<Window>,
}
impl Clipboard {
    fn connect(window: Arc<Window>, view: &View) -> Self {
        // SAFETY: _window outlives native, including its display handle.
        let native = match unsafe { window_clipboard::Clipboard::connect(window.as_ref()) } {
            Ok(native) => Some(native),
            Err(error) => {
                view.0
                    .diagnostics
                    .borrow_mut()
                    .push(format!("system clipboard unavailable: {error}"));
                None
            }
        };
        Self {
            native,
            _window: window,
        }
    }
}
impl iced::advanced::Clipboard for Clipboard {
    fn read(&self, kind: iced::advanced::clipboard::Kind) -> Option<String> {
        let clipboard = self.native.as_ref()?;
        match kind {
            iced::advanced::clipboard::Kind::Standard => clipboard.read().ok(),
            iced::advanced::clipboard::Kind::Primary => {
                clipboard.read_primary().and_then(Result::ok)
            }
        }
    }
    fn write(&mut self, kind: iced::advanced::clipboard::Kind, value: String) {
        let Some(clipboard) = self.native.as_mut() else {
            return;
        };
        let result = match kind {
            iced::advanced::clipboard::Kind::Standard => clipboard.write(value),
            iced::advanced::clipboard::Kind::Primary => {
                clipboard.write_primary(value).unwrap_or(Ok(()))
            }
        };
        if let Err(error) = result {
            eprintln!("wakui2-tao clipboard write: {error}");
        }
    }
}
struct Painter {
    renderer: iced_tiny_skia::Renderer,
    cache: Cache,
    clipboard: Clipboard,
    cursor: mouse::Cursor,
    modifiers: iced::keyboard::Modifiers,
    deadline: Option<Instant>,
    ime_cursor: Option<iced::Point>,
}
impl Painter {
    fn new(window: Arc<Window>, view: &View) -> Self {
        Self {
            renderer: iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0)),
            cache: Cache::default(),
            clipboard: Clipboard::connect(window, view),
            cursor: mouse::Cursor::Unavailable,
            modifiers: Default::default(),
            deadline: None,
            ime_cursor: None,
        }
    }
    fn pump(&mut self, view: &View, bounds: Size, event: iced::Event, draw: bool) -> Vec<Message> {
        use iced_winit::core::window::RedrawRequest;
        if matches!(
            event,
            iced::Event::Window(iced::window::Event::RedrawRequested(_))
        ) {
            self.deadline = None;
        }
        let root = &view.0.root.0;
        let content = iced::widget::container(super::layout::Panel::element(
            wakui2_runtime::Layout::Stack {
                horizontal: false,
                gap: 0.0,
            },
            vec![(root.key(), root.style.get(), root.element())],
        ))
        .width(iced::Length::Fill);
        let mut ui = UserInterface::build(
            content,
            bounds,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        // Automation/bound viewport changes scroll the real iced widget, not just row data.
        for (id, offset) in view.0.scroll.take() {
            use iced::advanced::widget::{Id, operation::scrollable};
            let mut operation = scrollable::scroll_to::<()>(
                Id::from(id),
                scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(offset),
                },
            );
            ui.operate(&self.renderer, &mut operation);
        }
        let mut messages = Vec::new();
        let (state, _) = ui.update(
            &[event],
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        if let State::Updated { input_method, .. } = &state {
            self.ime_cursor = match input_method {
                iced::advanced::input_method::InputMethod::Enabled { cursor, .. } => {
                    Some(iced::Point::new(cursor.x, cursor.y + cursor.height))
                }
                _ => None,
            };
        }
        match state {
            State::Updated {
                redraw_request: RedrawRequest::At(at),
                ..
            } => {
                self.deadline = Some(self.deadline.map_or(at, |old| old.min(at)));
            }
            State::Updated {
                redraw_request: RedrawRequest::NextFrame,
                ..
            }
            | State::Outdated => self.deadline = Some(Instant::now()),
            _ => {}
        }
        if draw {
            ui.draw(
                &mut self.renderer,
                &iced::Theme::Light,
                &iced::advanced::renderer::Style::default(),
                self.cursor,
            );
            view.0.frames.set(view.0.frames.get() + 1);
        }
        self.cache = ui.into_cache();
        messages
    }
}
fn dispatch(view: &View, messages: Vec<Message>) {
    for message in messages {
        let _ = view.dispatch(message);
        if view.0.closed.get() {
            break;
        }
    }
}
fn fail(view: &View, error: String) {
    view.0.report(error);
    view.close();
}
struct Desktop {
    window: Arc<Window>,
    view: View,
    painter: Rc<RefCell<Painter>>,
    #[cfg(target_os = "linux")]
    area: gtk::DrawingArea,
    #[cfg(target_os = "linux")]
    signals: Vec<gtk::glib::SignalHandlerId>,
    #[cfg(not(target_os = "linux"))]
    compositor: Compositor,
    #[cfg(not(target_os = "linux"))]
    surface: Surface,
    #[cfg(not(target_os = "linux"))]
    size: (u32, u32),
}
impl Desktop {
    fn open(
        target: &tao::event_loop::EventLoopWindowTarget<Native>,
        view: &View,
        proxy: Proxy,
    ) -> Result<Self, String> {
        let window = Arc::new(
            WindowBuilder::new()
                .with_title(view.title())
                .with_visible(false)
                .with_inner_size(tao::dpi::LogicalSize::new(660.0, 520.0))
                .build(target)
                .map_err(|e| e.to_string())?,
        );
        let painter = Rc::new(RefCell::new(Painter::new(window.clone(), view)));
        #[cfg(target_os = "linux")]
        {
            let vbox = window
                .default_vbox()
                .ok_or("Tao GTK3 window has no default vbox")?;
            view.0
                .menus
                .init_for_gtk_window(window.gtk_window(), Some(vbox))
                .map_err(|e| e.to_string())?;
            let area = gtk::DrawingArea::new();
            area.set_has_window(false);
            area.set_can_focus(true);
            area.set_hexpand(true);
            area.set_vexpand(true);
            vbox.pack_end(&area, true, true, 0);
            let out = painter.clone();
            let owner = view.clone();
            let draw = area.connect_draw(move |area, context| {
                if owner.0.closed.get() {
                    return gtk::glib::Propagation::Stop;
                }
                let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                    let scale = f64::from(area.scale_factor());
                    let (width, height) = (area.allocated_width(), area.allocated_height());
                    if width <= 0 || height <= 0 {
                        return Ok(());
                    }
                    let viewport = Viewport::with_physical_size(
                        Size::new(
                            (f64::from(width) * scale) as u32,
                            (f64::from(height) * scale) as u32,
                        ),
                        scale as f32,
                    );
                    let (mut pixels, messages) = {
                        let mut painter = out.borrow_mut();
                        let messages =
                            painter.pump(&owner, viewport.logical_size(), redraw(), true);
                        let pixels = iced_tiny_skia::window::compositor::screenshot(
                            &mut painter.renderer,
                            &viewport,
                            iced::Color::WHITE,
                        );
                        (pixels, messages)
                    };
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        let native =
                            u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]]).to_ne_bytes();
                        pixel.copy_from_slice(&native);
                    }
                    let size = viewport.physical_size();
                    let image = gtk::cairo::ImageSurface::create_for_data(
                        pixels,
                        gtk::cairo::Format::Rgb24,
                        size.width as i32,
                        size.height as i32,
                        (size.width * 4) as i32,
                    )
                    .map_err(|e| e.to_string())?;
                    image.set_device_scale(scale, scale);
                    context
                        .set_source_surface(&image, 0.0, 0.0)
                        .map_err(|e| e.to_string())?;
                    context.paint().map_err(|e| e.to_string())?;
                    dispatch(&owner, messages);
                    Ok(())
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => fail(&owner, e),
                    Err(p) => fail(&owner, panic_error(p)),
                }
                proxy.send(Native::Turn);
                gtk::glib::Propagation::Stop
            });
            area.show();
            Ok(Self {
                window,
                view: view.clone(),
                painter,
                area,
                signals: vec![draw],
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = proxy;
            #[cfg(target_os = "windows")]
            {
                use tao::platform::windows::WindowExtWindows;
                unsafe { view.0.menus.init_for_hwnd(window.hwnd() as isize) }
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(target_os = "macos")]
            view.0.menus.init_for_nsapp();
            let mut compositor =
                compositor::new(iced_tiny_skia::Settings::default(), window.clone());
            let size = window.inner_size();
            let surface = compositor.create_surface(window.clone(), size.width, size.height);
            Ok(Self {
                window,
                view: view.clone(),
                painter,
                compositor,
                surface,
                size: (size.width, size.height),
            })
        }
    }
    fn bounds(&self) -> Size {
        #[cfg(target_os = "linux")]
        {
            Size::new(
                self.area.allocated_width().max(0) as f32,
                self.area.allocated_height().max(0) as f32,
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            let s = self
                .window
                .inner_size()
                .to_logical::<f64>(self.window.scale_factor());
            Size::new(s.width as f32, s.height as f32)
        }
    }
    fn offset(&self) -> iced::Point {
        #[cfg(target_os = "linux")]
        {
            let (x, y) = self
                .area
                .translate_coordinates(self.window.gtk_window(), 0, 0)
                .unwrap_or_default();
            iced::Point::new(x as f32, y as f32)
        }
        #[cfg(not(target_os = "linux"))]
        {
            iced::Point::ORIGIN
        }
    }
    fn refresh(&self) {
        self.window.set_title(&self.view.title());
        self.window
            .set_visible(self.view.0.visible.get() && !self.view.0.closed.get());
        if let Some(cursor) = self.painter.borrow().ime_cursor {
            let offset = self.offset();
            self.window.set_ime_position(tao::dpi::LogicalPosition::new(
                cursor.x + offset.x,
                cursor.y + offset.y,
            ));
        }
        #[cfg(target_os = "linux")]
        self.area.queue_draw();
        #[cfg(not(target_os = "linux"))]
        self.window.request_redraw();
    }
    fn event(&self, event: &WindowEvent<'_>) {
        #[cfg(target_os = "linux")]
        if self.painter.borrow().cursor.position().is_some()
            && matches!(
                event,
                WindowEvent::MouseInput {
                    state: tao::event::ElementState::Pressed,
                    ..
                }
            )
        {
            self.area.grab_focus();
        }
        let messages = {
            let mut painter = self.painter.borrow_mut();
            let Painter {
                cursor, modifiers, ..
            } = &mut *painter;
            let mapped = input::convert(
                event,
                self.window.scale_factor(),
                self.offset(),
                cursor,
                modifiers,
            );
            mapped
                .map(|e| painter.pump(&self.view, self.bounds(), e, false))
                .unwrap_or_default()
        };
        dispatch(&self.view, messages);
        self.refresh();
    }
    #[cfg(not(target_os = "linux"))]
    fn draw(&mut self) -> Result<(), String> {
        let s = self.window.inner_size();
        if s.width == 0 || s.height == 0 {
            return Ok(());
        }
        if self.size != (s.width, s.height) {
            self.compositor
                .configure_surface(&mut self.surface, s.width, s.height);
            self.size = (s.width, s.height);
        }
        let viewport = Viewport::with_physical_size(
            Size::new(s.width, s.height),
            self.window.scale_factor() as f32,
        );
        let messages = {
            let mut painter = self.painter.borrow_mut();
            let messages = painter.pump(&self.view, viewport.logical_size(), redraw(), true);
            self.compositor
                .present(
                    &mut painter.renderer,
                    &mut self.surface,
                    &viewport,
                    iced::Color::WHITE,
                    || {},
                )
                .map_err(|e| e.to_string())?;
            messages
        };
        dispatch(&self.view, messages);
        Ok(())
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            for signal in self.signals.drain(..) {
                self.area.disconnect(signal);
            }
            if let Err(error) = super::menu::clear_accelerators(&self.view.0.menus.items()) {
                self.view.0.report(error);
            }
            let _ = self
                .view
                .0
                .menus
                .remove_for_gtk_window(self.window.gtk_window());
            unsafe {
                self.area.destroy();
            }
        }
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            unsafe {
                let _ = self
                    .view
                    .0
                    .menus
                    .remove_for_hwnd(self.window.hwnd() as isize);
            }
        }
        #[cfg(target_os = "macos")]
        self.view.0.menus.remove_for_nsapp();
    }
}
fn redraw() -> iced::Event {
    iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now()))
}
pub(super) fn run(make: impl FnOnce(Rc<dyn Executor>) -> View + 'static) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        let mut builder = EventLoopBuilder::<Native>::with_user_event();
        #[cfg(target_os = "windows")]
        let menu_owner = {
            use tao::platform::windows::EventLoopBuilderExtWindows;
            use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, TranslateAcceleratorW};
            let owner = Rc::new(RefCell::new(std::rc::Weak::<super::Inner>::new()));
            let hook = owner.clone();
            builder.with_msg_hook(move |message| {
                let inner = hook.borrow().upgrade();
                let Some(inner) = inner.filter(|i| !i.closed.get()) else {
                    return false;
                };
                // Query each time: dynamic menu snapshots may replace the HACCEL.
                let accelerator = inner.menus.haccel();
                if message.is_null() || accelerator == 0 {
                    return false;
                }
                // SAFETY: Tao provides a valid MSG for this synchronous hook; the
                // live View owns the menu table throughout TranslateAcceleratorW.
                unsafe {
                    let message = message.cast::<MSG>();
                    TranslateAcceleratorW((*message).hwnd, accelerator as _, message) != 0
                }
            });
            owner
        };
        let mut event_loop = builder.build();
        let proxy = Proxy(event_loop.create_proxy());
        let wake = proxy.clone();
        let executor = Rc::new(Scheduler::new(Arc::new(move |id| {
            wake.send(Native::Wake(id))
        })));
        let _executor_guard = ExecutorGuard(executor.clone());
        let session = Session {
            view: make(executor.clone()),
            executor,
        };
        let view = &session.view;
        #[cfg(target_os = "windows")]
        menu_owner.replace(Rc::downgrade(&view.0));
        if view.0.closed.get() {
            return view.error().map_or(Ok(()), Err);
        }
        let desktop = Desktop::open(&event_loop, view, proxy.clone())?;
        #[cfg(not(target_os = "linux"))]
        let mut desktop = desktop;
        MENU_PROXY.lock().unwrap().replace(proxy.clone());
        let _bridge = MenuBridge;
        MENU_INIT.call_once(|| {
            muda::MenuEvent::set_event_handler(Some(|event: muda::MenuEvent| {
                let proxy = MENU_PROXY.lock().unwrap().clone();
                if let Some(proxy) = proxy {
                    proxy.send(Native::Menu(event.id));
                }
            }))
        });
        let invalidate = proxy.clone();
        let owner = Rc::downgrade(&view.0);
        let pending = Rc::downgrade(&session.executor);
        view.0.invalidate.replace(Some(Rc::new(move || {
            if owner.upgrade().is_some_and(|inner| inner.closed.get())
                && let Some(executor) = pending.upgrade()
            {
                executor.close();
            }
            invalidate.send(Native::Refresh);
        })));
        view.present();
        desktop.refresh();
        let status = event_loop.run_return(|event, _, flow| {
            let result = catch_unwind(AssertUnwindSafe(|| match event {
                Event::UserEvent(Native::Wake(id)) => session.executor.ready(id),
                Event::UserEvent(Native::Refresh) => desktop.refresh(),
                Event::UserEvent(Native::Menu(id)) => {
                    let id = view.0.menu_ids.borrow().get(&id).cloned();
                    if let Some(id) = id {
                        let _ = view.dispatch(Message::Menu(id));
                    }
                }
                Event::WindowEvent {
                    window_id, event, ..
                } if window_id == desktop.window.id() => {
                    if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
                        view.close();
                    } else {
                        desktop.event(&event);
                    }
                }
                #[cfg(not(target_os = "linux"))]
                Event::RedrawRequested(id) if id == desktop.window.id() => {
                    if let Err(e) = desktop.draw() {
                        fail(view, e);
                    }
                }
                Event::MainEventsCleared if session.executor.turn() => proxy.send(Native::Turn),
                _ => {}
            }));
            if let Err(panic) = result {
                fail(view, panic_error(panic));
            }
            if view.0.closed.get() {
                session.executor.close();
                *flow = ControlFlow::Exit;
            } else {
                let (due, deadline) = {
                    let mut painter = desktop.painter.borrow_mut();
                    let due = painter.deadline.is_some_and(|at| at <= Instant::now());
                    if due {
                        painter.deadline = None;
                    }
                    (due, painter.deadline)
                };
                if due {
                    desktop.refresh();
                }
                *flow = deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
            }
        });
        view.close();
        if let Some(error) = view.error() {
            return Err(error);
        }
        if status != 0 {
            return Err(format!(
                "Tao event loop exited with {status} (display disconnected)"
            ));
        }
        Ok(())
    }))
    .map_err(panic_error)?
}
