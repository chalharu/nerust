use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced_winit::program;
use tao::event_loop::EventLoopWindowTarget;
use wakui_core::scene::control::{ControlEvent, ControlScreen};
use wakui_core::{chrome::mailbox::WindowMailbox, chrome::window::Window};

use crate::aux_windows::shell::{AuxShell, AuxSurface};
use crate::control_binding::{ControlsBinding, EventQueue, Message, SharedScreen};
use crate::window_program::{WidgetBinding, WindowProgram};

/// A native window owns only surface/shell and typed widget signals.
pub(crate) struct WidgetWindow<B, S>
where
    B: WidgetBinding<Theme = iced::Theme, Renderer = iced_tiny_skia::Renderer>,
{
    shell: AuxShell<WindowProgram<B>>,
    signals: S,
}

pub(crate) trait WidgetSignals<B: WidgetBinding> {
    fn close_flag(&self) -> &Arc<AtomicBool>;
    fn refresh_flag(&self) -> &AtomicBool;
    fn capture(&self, _event: &iced::Event) -> Option<B::Message> {
        None
    }
}

impl<B, S> WidgetWindow<B, S>
where
    B: WidgetBinding<Theme = iced::Theme, Renderer = iced_tiny_skia::Renderer>,
    S: WidgetSignals<B>,
{
    fn open(
        window: wakui_core::chrome::window::Window,
        binding: B,
        event_loop: &EventLoopWindowTarget<crate::event_sink::UserEvent>,
        signals: S,
        size: (f64, f64),
    ) -> Option<Self> {
        let surface = AuxSurface::open(
            event_loop,
            &window.title,
            size,
            None,
            None,
            &window.title,
            Arc::clone(signals.close_flag()),
        )
        .map_err(|error| {
            // Typed scaffold failure: surface the tagged native detail here.
            log::error!("{error}");
            error
        })
        .ok()?;
        // Birth geometry is a placeholder on some compositors; record
        // it to tell "born 0x0, never resized" apart from later traffic.
        let (viewport_w, viewport_h) = surface.viewport();
        log::debug!(
            "widget create: physical={viewport_w}x{viewport_h} scale={} bounds={:?}",
            surface.scale(),
            surface.bounds(),
        );
        let (instance, _task) = program::Instance::new(WindowProgram {
            window,
            program: binding,
        });
        let shell = AuxShell::assemble(surface, instance);
        Some(Self { shell, signals })
    }
}

impl<B, S> crate::aux_windows::focus::AuxShellRef for WidgetWindow<B, S>
where
    B: WidgetBinding<Theme = iced::Theme, Renderer = iced_tiny_skia::Renderer>,
{
    fn shell(&self) -> &dyn crate::aux_windows::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux_windows::shell::AuxShellOps {
        &mut self.shell
    }
}

impl<B, S> crate::aux_windows::focus::AuxEvents for WidgetWindow<B, S>
where
    B: WidgetBinding<Theme = iced::Theme, Renderer = iced_tiny_skia::Renderer>,
    S: WidgetSignals<B>,
{
    fn handle(&mut self, mapped: iced::Event) -> bool {
        let extra = self.signals.capture(&mapped);
        self.shell
            .pump_finish(mapped, extra, self.signals.refresh_flag())
    }
}

pub(crate) struct WindowSignals {
    screen: SharedScreen,
    queue: EventQueue,
    mailbox: WindowMailbox,
    should_close: Arc<AtomicBool>,
    view_invalidated: AtomicBool,
}

impl WindowSignals {
    /// Take queued control events for the host drain.
    pub(crate) fn drain_events(&mut self) -> Vec<ControlEvent> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Push the latest app screen; the next rebuild renders it.
    pub(crate) fn push_screen(&self, screen: &ControlScreen) -> &AtomicBool {
        *self.screen.borrow_mut() = screen.clone();
        self.view_invalidated.store(true, Ordering::Release);
        &self.view_invalidated
    }
}

pub(crate) type ControlsWindowHandle = WidgetWindow<ControlsBinding, WindowSignals>;

impl ControlsWindowHandle {
    pub(crate) fn new(
        window: Window,
        screen: Box<ControlScreen>,
        mailbox: WindowMailbox,
        event_loop: &EventLoopWindowTarget<crate::event_sink::UserEvent>,
    ) -> Option<Self> {
        let wakui_core::chrome::window::WindowId::Auxiliary(_) = window.id else {
            unreachable!("widget windows are auxiliary windows");
        };
        let (binding, screen, queue) = ControlsBinding::new(*screen);
        let signals = WindowSignals {
            screen,
            queue,
            mailbox,
            should_close: Default::default(),
            view_invalidated: AtomicBool::new(false),
        };
        Self::open(window, binding, event_loop, signals, (960.0, 720.0))
    }
}

impl crate::aux_windows::registry::AuxController for ControlsWindowHandle {
    fn as_widget(&mut self) -> Option<&mut dyn crate::aux_windows::registry::WidgetSessionPort> {
        Some(self)
    }
    fn widget_mailbox(&self) -> Option<WindowMailbox> {
        Some(self.signals.mailbox.clone())
    }
}

impl crate::aux_windows::registry::WidgetSessionPort for ControlsWindowHandle {
    fn push_snapshot(&mut self, screen: &ControlScreen) {
        let flag = self.signals.push_screen(screen);
        self.shell.process([], flag);
    }

    fn drain_events(&mut self) -> Vec<ControlEvent> {
        self.signals.drain_events()
    }
}

impl WidgetSignals<ControlsBinding> for WindowSignals {
    fn close_flag(&self) -> &Arc<AtomicBool> {
        &self.should_close
    }
    fn refresh_flag(&self) -> &AtomicBool {
        &self.view_invalidated
    }
    fn capture(&self, mapped: &iced::Event) -> Option<Message> {
        if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = mapped
        {
            crate::control_binding::keyboard_key_from_physical(*physical_key)
                .map(Message::capture_key)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Event, Point, keyboard, mouse};
    use std::sync::atomic::AtomicBool;
    use tao::{dpi::PhysicalPosition, event::WindowEvent};

    use crate::aux_windows::event_adapt::adapt_event;
    use crate::input::convert::{FeedContext, feed_domain_event};

    #[test]
    fn widget_signals_keep_close_separate_from_repaint_and_clear() {
        use crate::aux_windows::widgets::bindings::tests::{mailbox_of, prepare};
        let mut support = crate::event_loop::host::TestHost::new().host;
        let id = prepare(&mut support, crate::testkit::TestAux::Beta);
        let settings = WindowSignals {
            screen: std::rc::Rc::new(std::cell::RefCell::new(
                support.core.widget_screen(id).expect("screen"),
            )),
            queue: Default::default(),
            mailbox: mailbox_of(&support, id),
            should_close: Default::default(),
            view_invalidated: AtomicBool::new(false),
        };
        settings.close_flag().store(true, Ordering::Release);
        settings.refresh_flag().store(true, Ordering::Release);
        assert!(settings.refresh_flag().swap(false, Ordering::AcqRel));
        assert!(settings.close_flag().load(Ordering::Acquire));

        settings.mailbox.request_close();
        settings.mailbox.invalidate();
        assert!(settings.mailbox.take_invalidated());
        assert!(settings.mailbox.close_requested());
        assert!(!settings.mailbox.has_pending());
    }

    fn initial_cursor() -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(0.0, 0.0))
    }

    fn feed(
        event: WindowEvent,
        cursor: &mut mouse::Cursor,
        modifiers: &mut keyboard::Modifiers,
        should_close: &AtomicBool,
    ) -> Option<Event> {
        feed_domain_event(
            adapt_event(&event),
            &mut FeedContext {
                cursor,
                modifiers,
                scale_factor: 1.0,
                should_close,
            },
        )
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
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
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
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
        assert!(result.is_none());
        assert!(matches!(cursor, mouse::Cursor::Unavailable));
    }

    #[test]
    fn close_requested_sets_flag() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        let event = WindowEvent::CloseRequested;
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
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
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
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
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
        assert!(result.is_none());
    }

    #[test]
    fn unhandled_events_return_none() {
        let mut cursor = initial_cursor();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        // Focused is not handled by settings UI (falls through to _ => None).
        let event = WindowEvent::Focused(true);
        let result = feed(event, &mut cursor, &mut modifiers, &should_close);
        assert!(result.is_none());
    }
}
