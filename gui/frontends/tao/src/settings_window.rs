use std::sync::{Arc, Mutex, atomic::AtomicBool};

use iced::{Event, Point, Size, advanced::renderer, keyboard, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    core::SmolStr,
    graphics::Viewport,
    program,
    runtime::user_interface::{Cache, UserInterface},
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::settings::editor::CaptureTarget;
#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::{
    event_loop::EventLoopWindowTarget,
    keyboard::ModifiersState as TaoModifiers,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::{
    settings::ui::{Message, SettingsAppProgram},
    tao_conversions::*,
};

/// Owns Instance + Cache + UI, ensuring UI is dropped before Instance.
/// This makes the phantom lifetime safety explicit via Drop rather than
/// relying on struct field declaration order.
pub(crate) struct UiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<SettingsAppProgram>,
}

impl UiState {
    /// Build a UserInterface from instance state, transmuting the phantom
    /// lifetime to 'static. Safe because UiState's Drop ensures the UI is
    /// destroyed before the Instance it references.
    #[allow(clippy::missing_transmute_annotations)]
    fn build_ui(
        instance: &program::Instance<SettingsAppProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer> {
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

    fn new(
        instance: program::Instance<SettingsAppProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> Self {
        let ui = Self::build_ui(&instance, window_id, bounds, Cache::default(), renderer);
        Self {
            ui: std::mem::ManuallyDrop::new(ui),
            instance,
        }
    }

    fn ui_mut(
        &mut self,
    ) -> &mut UserInterface<'static, Message, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }

    /// Process messages, then rebuild UI with updated instance + old cache.
    fn process_messages(
        &mut self,
        messages: Vec<Message>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        if messages.is_empty() {
            return;
        }
        // Step 1: Replace UI with a placeholder, extract old cache.
        let placeholder = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(
                &self.instance,
                window_id,
                bounds,
                Cache::default(),
                renderer,
            ),
        );
        let cache = placeholder.into_cache();

        // Step 2: Process messages (mutate instance state).
        // Note: SettingsAppState::update() always returns Task::none().
        // If a future handler returns a non-empty Task, it must be executed
        // (e.g., via block_on) rather than discarded here.
        for msg in messages {
            let _task = self.instance.update(msg);
        }

        // Step 3: Replace placeholder with real UI from new state + cache.
        let stale = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(&self.instance, window_id, bounds, cache, renderer),
        );
        let _ = stale.into_cache(); // discard placeholder
    }
}

impl Drop for UiState {
    fn drop(&mut self) {
        // SAFETY: UI has a phantom lifetime tied to Instance.
        // Dropping UI before Instance prevents dangling reference.
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct SettingsWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: UiState,
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
    backend: Renderer,
}

impl SettingsRenderer {
    fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
    ) -> Result<(), iced_tiny_skia::graphics::compositor::SurfaceError> {
        self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        )
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }
}

impl SettingsWindowHandle {
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let pending_apply = Arc::new(Mutex::new(None));
        let capture_target = Arc::new(Mutex::new(None));

        #[allow(unused_mut)]
        let mut wb = WindowBuilder::new()
            .with_title("Preferences")
            .with_inner_size(tao::dpi::LogicalSize::new(960.0, 720.0));
        #[cfg(target_os = "macos")]
        {
            wb = wb.with_automatic_window_tabbing(false);
        }
        let window = Arc::new(match wb.build(event_loop) {
            Ok(w) => w,
            Err(e) => {
                log::error!("failed to create settings window: {e}");
                return None;
            }
        });
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
        let bounds = Viewport::with_physical_size(
            Size::new(viewport_physical.0, viewport_physical.1),
            scale_factor,
        )
        .logical_size();
        let ui_state = UiState::new(instance, window_id, bounds, &mut renderer);

        window.request_redraw();

        Some(Self {
            window,
            window_id,
            ui_state,
            renderer: SettingsRenderer {
                compositor,
                surface,
                backend: renderer,
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
        })
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

        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut messages,
        );

        if !messages.is_empty() {
            let bounds = Viewport::with_physical_size(
                Size::new(self.viewport_physical.0, self.viewport_physical.1),
                self.scale_factor,
            )
            .logical_size();
            self.ui_state.process_messages(
                messages,
                self.window_id,
                bounds,
                &mut self.renderer.backend,
            );
        }
    }

    pub(crate) fn render(&mut self) {
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let vp = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );

        // update() with RedrawRequested to refresh widget status (hover state).
        // Messages produced here (if any) are discarded; no standard iced widget
        // generates messages on RedrawRequested.
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = self.ui_state.ui_mut().update(
            &[redraw_event],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui_state.ui_mut().draw(
            &mut self.renderer.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );

        if let Err(e) = self.renderer.present(&vp, iced::Color::BLACK) {
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
        if let Some(iced_event) = convert_tao_window_event(
            event,
            &mut self.cursor,
            self.scale_factor,
            &mut self.modifiers,
            &self.should_close,
        ) {
            self.handle_event(iced_event);
        }
    }
}

/// Convert Tao WindowEvent to iced Event, updating cursor/modifiers/should_close.
/// Returns Some(event) for events that should be forwarded to handle_event(),
/// None for events that are fully handled here (CursorLeft, CloseRequested, etc.).
#[allow(deprecated)]
fn convert_tao_window_event(
    event: tao::event::WindowEvent,
    cursor: &mut mouse::Cursor,
    scale_factor: f32,
    modifiers: &mut keyboard::Modifiers,
    should_close: &AtomicBool,
) -> Option<iced::Event> {
    use tao::event::WindowEvent;
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let logical = position.to_logical::<f64>(scale_factor as f64);
            let point = Point::new(logical.x as f32, logical.y as f32);
            *cursor = mouse::Cursor::Available(point);
            Some(Event::Mouse(mouse::Event::CursorMoved { position: point }))
        }
        WindowEvent::CursorLeft { .. } => {
            *cursor = mouse::Cursor::Unavailable;
            None
        }
        WindowEvent::KeyboardInput { event: ke, .. } => {
            let iced_key = tao_key_to_iced_key(&ke.logical_key);
            let physical_key =
                keyboard::key::Physical::Code(tao_keycode_to_iced_code(ke.physical_key));
            match ke.state {
                tao::event::ElementState::Pressed => {
                    Some(Event::Keyboard(keyboard::Event::KeyPressed {
                        key: iced_key.clone(),
                        modified_key: iced_key.clone(),
                        physical_key,
                        modifiers: *modifiers,
                        location: keyboard::Location::Standard,
                        text: ke.text.map(SmolStr::new),
                        repeat: ke.repeat,
                    }))
                }
                tao::event::ElementState::Released => {
                    Some(Event::Keyboard(keyboard::Event::KeyReleased {
                        key: iced_key.clone(),
                        modified_key: iced_key,
                        physical_key,
                        modifiers: *modifiers,
                        location: keyboard::Location::Standard,
                    }))
                }
                _ => None,
            }
        }
        WindowEvent::MouseInput { button, state, .. } => {
            let btn = match button {
                tao::event::MouseButton::Left => mouse::Button::Left,
                tao::event::MouseButton::Right => mouse::Button::Right,
                tao::event::MouseButton::Middle => mouse::Button::Middle,
                _ => return None,
            };
            match state {
                tao::event::ElementState::Pressed => {
                    Some(Event::Mouse(mouse::Event::ButtonPressed(btn)))
                }
                tao::event::ElementState::Released => {
                    Some(Event::Mouse(mouse::Event::ButtonReleased(btn)))
                }
                _ => None,
            }
        }
        WindowEvent::ModifiersChanged(state) => {
            *modifiers = tao_modifiers_to_iced(state);
            None
        }
        WindowEvent::CloseRequested => {
            should_close.store(true, std::sync::atomic::Ordering::Release);
            None
        }
        // Touch, IME, axis motion, and other platform-specific events
        // are not needed for the settings UI.
        _ => None,
    }
}

impl SettingsWindowHandle {
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.viewport = Size::new(
            width as f32 / self.scale_factor,
            height as f32 / self.scale_factor,
        );
        self.renderer.resize(width, height);
    }
}

#[cfg(test)]
mod tests {
    use tao::{dpi::PhysicalPosition, event::WindowEvent};

    use super::*;

    fn initial_cursor() -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(0.0, 0.0))
    }

    // Helper: Tao's MouseInput/CursorMoved/etc require a deprecated modifiers field.
    fn dummy_mods() -> tao::keyboard::ModifiersState {
        tao::keyboard::ModifiersState::default()
    }

    // SAFETY: Tao's DeviceId::dummy() is unsafe but stable across versions.
    // We call it once here rather than inlining unsafe in every test.
    fn dummy_dev() -> tao::event::DeviceId {
        unsafe { tao::event::DeviceId::dummy() }
    }

    #[test]
    fn cursor_moved_updates_cursor_and_returns_event() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::CursorMoved {
            device_id: dummy_dev(),
            position: PhysicalPosition::new(200.0, 300.0),
            modifiers: dummy_mods(),
        };
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            Event::Mouse(mouse::Event::CursorMoved { .. })
        ));
        assert!(!should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn cursor_left_sets_unavailable() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::CursorLeft {
            device_id: dummy_dev(),
        };
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_none());
        assert!(matches!(cursor, mouse::Cursor::Unavailable));
    }

    #[test]
    fn close_requested_sets_flag() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::CloseRequested;
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_none());
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn mouse_input_pressed() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::MouseInput {
            device_id: dummy_dev(),
            state: tao::event::ElementState::Pressed,
            button: tao::event::MouseButton::Left,
            modifiers: dummy_mods(),
        };
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ));
    }

    #[test]
    fn mouse_other_button_ignored() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::MouseInput {
            device_id: dummy_dev(),
            state: tao::event::ElementState::Pressed,
            button: tao::event::MouseButton::Other(5),
            modifiers: dummy_mods(),
        };
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_none());
    }

    #[test]
    fn unhandled_events_return_none() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        // Focused is not handled by settings UI (falls through to _ => None)
        let event = WindowEvent::Focused(true);
        let result =
            convert_tao_window_event(event, &mut cursor, 1.0, &mut modifiers, &should_close);
        assert!(result.is_none());
    }
}
