use crate::settings::ui::{Message, SettingsAppProgram};
use iced::advanced::renderer;
use iced::keyboard;
use iced::mouse;
use iced::theme;
use iced::{Event, Font, Point, Size};
use iced_tiny_skia::Renderer;
use iced_tiny_skia::graphics::compositor::Compositor as _;
use iced_tiny_skia::window::compositor;
use iced_tiny_skia::window::{Compositor, Surface};
use iced_winit::Clipboard;
use iced_winit::core::SmolStr;
use iced_winit::graphics::Viewport;
use iced_winit::program;
use iced_winit::runtime::user_interface::{Cache, UserInterface};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::settings::editor::CaptureTarget;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopWindowTarget;
use tao::keyboard::ModifiersState as TaoModifiers;
#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::window::{Window as TaoWindow, WindowBuilder};

pub(crate) struct SettingsWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    // ui before instance: correct drop order (phantom lifetime)
    ui: UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer>,
    instance: program::Instance<SettingsAppProgram>,
    renderer: SettingsRenderer,
    viewport: Size,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) capture_target: Arc<Mutex<Option<CaptureTarget>>>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

pub(crate) struct SettingsRenderer {
    compositor: Compositor,
    surface: Surface,
    renderer: Renderer,
}

impl SettingsWindowHandle {
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Self {
        let should_close = Arc::new(AtomicBool::new(false));
        let pending_apply = Arc::new(Mutex::new(None));
        let capture_target = Arc::new(Mutex::new(None));

        #[allow(unused_mut)]
        let mut window = WindowBuilder::new()
            .with_title("Preferences")
            .with_inner_size(tao::dpi::LogicalSize::new(960.0, 720.0));
        #[cfg(target_os = "macos")]
        {
            window = window.with_automatic_window_tabbing(false);
        }
        let window = Arc::new(window.build(event_loop).unwrap());
        let window_id = iced::window::Id::unique();

        let program = SettingsAppProgram {
            snapshot,
            should_close: should_close.clone(),
            pending_apply: pending_apply.clone(),
            capture_target: capture_target.clone(),
        };
        let (instance, _task) = program::Instance::new(program);
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);
        let logical_size = window_size.to_logical::<f64>(scale_factor as f64);
        let viewport = Size::new(logical_size.width as f32, logical_size.height as f32);

        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let mut renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);

        // Eagerly create the UserInterface so ensure_ui() is not needed.
        let vp = Viewport::with_physical_size(
            Size::new(viewport_physical.0, viewport_physical.1),
            scale_factor,
        );
        let ui = transmute_build_ui(
            &instance,
            window_id,
            vp.logical_size(),
            Cache::default(),
            &mut renderer,
        );

        window.request_redraw();

        Self {
            window,
            window_id,
            ui,
            instance,
            renderer: SettingsRenderer {
                compositor,
                surface,
                renderer,
            },
            viewport,
            viewport_physical,
            scale_factor,
            modifiers: keyboard::Modifiers::default(),
            pending_apply,
            should_close,
            capture_target,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        }
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        let mut messages = Vec::new();

        {
            let capture_guard = self.capture_target.lock().unwrap();
            if capture_guard.is_some()
                && let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    physical_key,
                    repeat: false,
                    ..
                }) = &mapped
                && let Some(key) = crate::settings::ui::keyboard_key_from_physical(*physical_key)
            {
                drop(capture_guard);
                messages.push(Message::CaptureKey(key));
            }
        }

        let _state = self.ui.update(
            &[mapped],
            self.cursor,
            &mut self.renderer.renderer,
            &mut self.clipboard,
            &mut messages,
        );

        if !messages.is_empty() {
            // Step 1: Extract old UI, insert temporary UI (placeholder).
            // The placeholder is from the pre-message state and is never
            // displayed; it's immediately replaced in step 3.
            let window_id = self.window_id;
            let bounds = Viewport::with_physical_size(
                Size::new(self.viewport_physical.0, self.viewport_physical.1),
                self.scale_factor,
            )
            .logical_size();
            let renderer = &mut self.renderer.renderer;
            let old_ui = std::mem::replace(
                &mut self.ui,
                transmute_build_ui(
                    &self.instance,
                    window_id,
                    bounds,
                    Cache::default(),
                    renderer,
                ),
            );
            let cache = old_ui.into_cache();

            // Step 2: Process messages (mutate instance state)
            for msg in messages {
                let _task = self.instance.update(msg);
            }

            // Step 3: Replace placeholder with real UI from NEW state + cache
            let window_id = self.window_id;
            let bounds = Viewport::with_physical_size(
                Size::new(self.viewport_physical.0, self.viewport_physical.1),
                self.scale_factor,
            )
            .logical_size();
            let renderer = &mut self.renderer.renderer;
            let placeholder = std::mem::replace(
                &mut self.ui,
                transmute_build_ui(&self.instance, window_id, bounds, cache, renderer),
            );
            let _ = placeholder.into_cache(); // discard placeholder
        }
    }

    pub(crate) fn render(&mut self) {
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let vp = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );

        // update() with RedrawRequested to refresh widget status (hover state)
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = self.ui.update(
            &[redraw_event],
            self.cursor,
            &mut self.renderer.renderer,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui.draw(
            &mut self.renderer.renderer,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );

        if let Err(e) = self.renderer.compositor.present(
            &mut self.renderer.renderer,
            &mut self.renderer.surface,
            &vp,
            iced::Color::BLACK,
            || {},
        ) {
            log::warn!("settings render present failed: {e:?}");
        }
    }

    pub(crate) fn take_pending_apply(&mut self) -> Option<SettingsSnapshot> {
        self.pending_apply.lock().unwrap().take()
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: TaoModifiers) {
        self.modifiers = tao_modifiers_to_iced(modifiers);
    }

    pub(crate) fn update_modifiers_from_tao_event(&mut self, event: &tao::event::WindowEvent) {
        if let tao::event::WindowEvent::ModifiersChanged(state) = event {
            self.set_modifiers(*state);
        }
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        use tao::event::WindowEvent;
        let iced_event = match event {
            WindowEvent::CursorMoved { position, .. } => {
                let logical = position.to_logical::<f64>(self.scale_factor as f64);
                let point = Point::new(logical.x as f32, logical.y as f32);
                self.cursor = mouse::Cursor::Available(point);
                Event::Mouse(mouse::Event::CursorMoved { position: point })
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = mouse::Cursor::Unavailable;
                return;
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                let modifiers = self.modifiers;
                let iced_key = tao_key_to_iced_key(&ke.logical_key);
                let physical_key =
                    keyboard::key::Physical::Code(tao_keycode_to_iced_code(ke.physical_key));
                match ke.state {
                    tao::event::ElementState::Pressed => {
                        Event::Keyboard(keyboard::Event::KeyPressed {
                            key: iced_key.clone(),
                            modified_key: iced_key.clone(),
                            physical_key,
                            modifiers,
                            location: keyboard::Location::Standard,
                            text: ke.text.map(SmolStr::new),
                            repeat: ke.repeat,
                        })
                    }
                    tao::event::ElementState::Released => {
                        Event::Keyboard(keyboard::Event::KeyReleased {
                            key: iced_key.clone(),
                            modified_key: iced_key,
                            physical_key,
                            modifiers,
                            location: keyboard::Location::Standard,
                        })
                    }
                    _ => return,
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let btn = match button {
                    tao::event::MouseButton::Left => mouse::Button::Left,
                    tao::event::MouseButton::Right => mouse::Button::Right,
                    tao::event::MouseButton::Middle => mouse::Button::Middle,
                    _ => return,
                };
                match state {
                    tao::event::ElementState::Pressed => {
                        Event::Mouse(mouse::Event::ButtonPressed(btn))
                    }
                    tao::event::ElementState::Released => {
                        Event::Mouse(mouse::Event::ButtonReleased(btn))
                    }
                    _ => return,
                }
            }
            WindowEvent::ModifiersChanged(state) => {
                self.set_modifiers(state);
                return;
            }
            WindowEvent::CloseRequested => {
                self.should_close
                    .store(true, std::sync::atomic::Ordering::Release);
                return;
            }
            // Touch, IME, axis motion, and other platform-specific events
            // are not needed for the settings UI.
            _ => return,
        };
        self.handle_event(iced_event);
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.viewport = Size::new(
            width as f32 / self.scale_factor,
            height as f32 / self.scale_factor,
        );
        self.renderer
            .compositor
            .configure_surface(&mut self.renderer.surface, width, height);
    }
}

// ---------------------------------------------------------------------------
// Helper: build UserInterface with lifetime transmute
//
// SAFETY: UserInterface<'a> has a phantom lifetime 'a that does not actually
// borrow any data after build(). The 'a on Box<dyn Widget<'a, ...>> is a
// type-erased upper bound that is not linked to any real borrow.
// Additionally, the ui field is declared before instance in
// SettingsWindowHandle, ensuring correct drop order (ui dropped first).
//
// self_cell v1.2.2 cannot replace this because its macro requires
// $Dependent:ident and appends <'static> automatically, which conflicts
// with UserInterface already having a lifetime parameter.
#[allow(clippy::missing_transmute_annotations)]
fn transmute_build_ui(
    instance: &program::Instance<SettingsAppProgram>,
    window_id: iced::window::Id,
    bounds: Size,
    cache: Cache,
    renderer: &mut iced_tiny_skia::Renderer,
) -> UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer> {
    // SAFETY: UserInterface<'a> has a phantom lifetime. After build(), 'a
    // does not borrow any data. The caller ensures the returned UI is
    // stored in a field that is dropped before the instance.
    unsafe {
        std::mem::transmute::<
            UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer>,
            UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer>,
        >(UserInterface::build(
            instance.view(window_id),
            bounds,
            cache,
            renderer,
        ))
    }
}

// ---------------------------------------------------------------------------
// Tao type conversions
// ---------------------------------------------------------------------------

fn tao_modifiers_to_iced(m: TaoModifiers) -> keyboard::Modifiers {
    let mut out = keyboard::Modifiers::empty();
    out.set(keyboard::Modifiers::SHIFT, m.contains(TaoModifiers::SHIFT));
    out.set(keyboard::Modifiers::CTRL, m.contains(TaoModifiers::CONTROL));
    out.set(keyboard::Modifiers::ALT, m.contains(TaoModifiers::ALT));
    out.set(keyboard::Modifiers::LOGO, m.contains(TaoModifiers::SUPER));
    out
}

fn tao_keycode_to_iced_code(code: tao::keyboard::KeyCode) -> keyboard::key::Code {
    use keyboard::key::Code as I;
    use tao::keyboard::KeyCode as T;
    match code {
        T::KeyA => I::KeyA,
        T::KeyB => I::KeyB,
        T::KeyC => I::KeyC,
        T::KeyD => I::KeyD,
        T::KeyE => I::KeyE,
        T::KeyF => I::KeyF,
        T::KeyG => I::KeyG,
        T::KeyH => I::KeyH,
        T::KeyI => I::KeyI,
        T::KeyJ => I::KeyJ,
        T::KeyK => I::KeyK,
        T::KeyL => I::KeyL,
        T::KeyM => I::KeyM,
        T::KeyN => I::KeyN,
        T::KeyO => I::KeyO,
        T::KeyP => I::KeyP,
        T::KeyQ => I::KeyQ,
        T::KeyR => I::KeyR,
        T::KeyS => I::KeyS,
        T::KeyT => I::KeyT,
        T::KeyU => I::KeyU,
        T::KeyV => I::KeyV,
        T::KeyW => I::KeyW,
        T::KeyX => I::KeyX,
        T::KeyY => I::KeyY,
        T::KeyZ => I::KeyZ,
        T::Digit0 => I::Digit0,
        T::Digit1 => I::Digit1,
        T::Digit2 => I::Digit2,
        T::Digit3 => I::Digit3,
        T::Digit4 => I::Digit4,
        T::Digit5 => I::Digit5,
        T::Digit6 => I::Digit6,
        T::Digit7 => I::Digit7,
        T::Digit8 => I::Digit8,
        T::Digit9 => I::Digit9,
        T::ArrowUp => I::ArrowUp,
        T::ArrowDown => I::ArrowDown,
        T::ArrowLeft => I::ArrowLeft,
        T::ArrowRight => I::ArrowRight,
        T::Enter => I::Enter,
        T::Escape => I::Escape,
        T::Space => I::Space,
        T::Tab => I::Tab,
        T::Backspace => I::Backspace,
        T::Delete => I::Delete,
        T::Insert => I::Insert,
        T::Home => I::Home,
        T::End => I::End,
        T::PageUp => I::PageUp,
        T::PageDown => I::PageDown,
        T::F1 => I::F1,
        T::F2 => I::F2,
        T::F3 => I::F3,
        T::F4 => I::F4,
        T::F5 => I::F5,
        T::F6 => I::F6,
        T::F7 => I::F7,
        T::F8 => I::F8,
        T::F9 => I::F9,
        T::F10 => I::F10,
        T::F11 => I::F11,
        T::F12 => I::F12,
        T::ShiftLeft | T::ShiftRight => I::ShiftLeft,
        T::ControlLeft | T::ControlRight => I::ControlLeft,
        T::AltLeft | T::AltRight => I::AltLeft,
        T::SuperLeft | T::SuperRight => I::SuperLeft,
        T::Numpad0 => I::Numpad0,
        T::Numpad1 => I::Numpad1,
        T::Numpad2 => I::Numpad2,
        T::Numpad3 => I::Numpad3,
        T::Numpad4 => I::Numpad4,
        T::Numpad5 => I::Numpad5,
        T::Numpad6 => I::Numpad6,
        T::Numpad7 => I::Numpad7,
        T::Numpad8 => I::Numpad8,
        T::Numpad9 => I::Numpad9,
        T::NumpadAdd => I::NumpadAdd,
        T::NumpadSubtract => I::NumpadSubtract,
        T::NumpadMultiply => I::NumpadMultiply,
        T::NumpadDivide => I::NumpadDivide,
        T::NumpadDecimal => I::NumpadDecimal,
        T::NumpadEnter => I::NumpadEnter,
        T::CapsLock => I::CapsLock,
        T::NumLock => I::NumLock,
        T::ScrollLock => I::ScrollLock,
        T::Comma => I::Comma,
        T::Period => I::Period,
        T::Semicolon => I::Semicolon,
        T::Quote => I::Quote,
        T::Backquote => I::Backquote,
        T::Minus => I::Minus,
        T::Equal => I::Equal,
        T::BracketLeft => I::BracketLeft,
        T::BracketRight => I::BracketRight,
        T::Backslash => I::Backslash,
        T::Slash => I::Slash,
        T::IntlBackslash => I::IntlBackslash,
        _ => I::Backquote,
    }
}

fn tao_key_to_iced_key(key: &tao::keyboard::Key) -> keyboard::Key {
    match key {
        tao::keyboard::Key::Character(s) => keyboard::Key::Character(SmolStr::new(s)),
        _ => keyboard::Key::Unidentified,
    }
}

fn default_font() -> Font {
    #[cfg(target_os = "windows")]
    {
        Font::with_name("Yu Gothic UI")
    }
    #[cfg(not(target_os = "windows"))]
    {
        Font::DEFAULT
    }
}
