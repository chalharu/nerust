use crate::settings::ui::{Message, SettingsAppProgram};
use iced::advanced::renderer;
use iced::theme;
use iced::{mouse, Font, Size};
use iced_tiny_skia::graphics::compositor::Compositor as _;
use iced_tiny_skia::window::compositor;
use iced_tiny_skia::window::{Compositor, Surface};
use iced_tiny_skia::Renderer;
use iced_winit::graphics::Viewport;
use iced_winit::program;
use iced_winit::Clipboard;
use iced_winit::runtime::user_interface::{Cache, UserInterface};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::settings::editor::CaptureTarget;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopWindowTarget;
use iced_winit::core::{Event, Point};
use iced_winit::core::mouse as IcedMouse;
use iced_winit::winit::keyboard::ModifiersState;
use tao::window::{Window as TaoWindow, WindowBuilder};

pub(crate) struct SettingsWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    instance: program::Instance<SettingsAppProgram>,
    ui_cache: Cache,
    renderer: SettingsRenderer,
    viewport: Size,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: ModifiersState,
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

        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Preferences")
                .with_inner_size(tao::dpi::LogicalSize::new(960.0, 720.0))
                .build(event_loop)
                .unwrap(),
        );
        let window_id = iced::window::Id::unique();

        let program = SettingsAppProgram {
            snapshot,
            should_close: should_close.clone(),
            pending_apply: pending_apply.clone(),
            capture_target: capture_target.clone(),
        };
        let (instance, _task) = program::Instance::new(program);

        let window_size = window.inner_size();
        let viewport = Size::new(window_size.width as f32, window_size.height as f32);
        let viewport_physical = (window_size.width, window_size.height);
        let ui_cache = Cache::default();

        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let renderer = compositor.create_renderer();
        let surface = compositor.create_surface(
            Arc::clone(&window),
            window_size.width,
            window_size.height,
        );

        let scale_factor = window.scale_factor() as f32;

        Self {
            window,
            window_id,
            instance,
            ui_cache,
            renderer: SettingsRenderer {
                compositor,
                surface,
                renderer,
            },
            viewport,
            viewport_physical,
            scale_factor,
            modifiers: ModifiersState::default(),
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
            if capture_guard.is_some() {
                if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    physical_key, repeat: false, ..
                }) = &mapped {
                    if let Some(key) = crate::settings::ui::keyboard_key_from_physical(*physical_key) {
                        drop(capture_guard);
                        messages.push(Message::CaptureKey(key));
                    }
                }
            }
        }

        let element = self.instance.view(self.window_id);
        let cache = std::mem::take(&mut self.ui_cache);
        let mut ui = UserInterface::build(
            element,
            self.viewport,
            cache,
            &mut self.renderer.renderer,
        );
        ui.update(
            &[mapped],
            self.cursor,
            &mut self.renderer.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.ui_cache = ui.into_cache();

        for msg in messages {
            let _task = self.instance.update(msg);
        }
    }

    pub(crate) fn render(&mut self) {
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let viewport = Viewport::with_physical_size(
            iced::Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );

        let element = self.instance.view(self.window_id);
        let cache = std::mem::take(&mut self.ui_cache);
        let mut ui = UserInterface::build(
            element,
            self.viewport,
            cache,
            &mut self.renderer.renderer,
        );
        ui.draw(
            &mut self.renderer.renderer,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        self.ui_cache = ui.into_cache();

        let _ = self.renderer.compositor.present(
            &mut self.renderer.renderer,
            &mut self.renderer.surface,
            &viewport,
            iced::Color::BLACK,
            || {},
        );
    }

    pub(crate) fn take_pending_apply(&mut self) -> Option<SettingsSnapshot> {
        self.pending_apply.lock().unwrap().take()
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        // SAFETY: tao::ModifiersState and winit::ModifiersState have same bit layout
        self.modifiers = unsafe { std::mem::transmute(modifiers) };
    }

    pub(crate) fn update_modifiers_from_tao_event(
        &mut self,
        event: &tao::event::WindowEvent,
    ) {
        if let tao::event::WindowEvent::ModifiersChanged(state) = event {
            self.set_modifiers(*state);
        }
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        use tao::event::WindowEvent;
        // Convert Tao WindowEvent to iced Event for UserInterface processing.
        // Only essential events are handled; Tao types differ from winit types
        // used by iced_winit::conversion::window_event.
        let iced_event = match event {
            WindowEvent::CursorMoved { position, .. } => {
                let logical = position.to_logical::<f64>(self.scale_factor as f64);
                let point = Point::new(logical.x as f32, logical.y as f32);
                self.cursor = IcedMouse::Cursor::Available(point);
                Event::Mouse(IcedMouse::Event::CursorMoved { position: point })
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = IcedMouse::Cursor::Unavailable;
                return;
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let btn = match button {
                    tao::event::MouseButton::Left => IcedMouse::Button::Left,
                    tao::event::MouseButton::Right => IcedMouse::Button::Right,
                    tao::event::MouseButton::Middle => IcedMouse::Button::Middle,
                    _ => return,
                };
                match state {
                    tao::event::ElementState::Pressed => {
                        Event::Mouse(IcedMouse::Event::ButtonPressed(btn))
                    }
                    tao::event::ElementState::Released => {
                        Event::Mouse(IcedMouse::Event::ButtonReleased(btn))
                    }
                    _ => return,
                }
            }
            WindowEvent::CloseRequested => {
                self.should_close.store(true, std::sync::atomic::Ordering::Release);
                return;
            }
            _ => return,
        };
        self.handle_event(iced_event);
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport = Size::new(width as f32, height as f32);
        self.viewport_physical = (width, height);
        self.renderer.compositor.configure_surface(
            &mut self.renderer.surface,
            width,
            height,
        );
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
