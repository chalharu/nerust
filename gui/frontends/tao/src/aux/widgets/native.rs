use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced_winit::program;
use nerust_frontend_host::settings::SettingsMailbox;
use nerust_gui_runtime::settings::SettingsSnapshot;
use tao::event_loop::EventLoopWindowTarget;

use super::bindings::{Message, SettingsBinding};
use super::history::{HistoryBinding, HistoryMessage};
use crate::aux::shell::{AuxShell, AuxSurface};
use crate::aux::window::{WidgetBinding, WindowProgram};
use nerust_gui_shell::session::osd::HistoryLine;
use nerust_settings_core::model::language::AppLanguage;

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
        window: nerust_frontend_host::window::Window,
        binding: B,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        signals: S,
        size: (f64, f64),
    ) -> Option<Self> {
        let surface = AuxSurface::open(
            event_loop,
            &window.title,
            size,
            None,
            None,
            B::KIND.log_tag(),
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

impl<B, S> crate::aux::focus::AuxShellRef for WidgetWindow<B, S>
where
    B: WidgetBinding<Theme = iced::Theme, Renderer = iced_tiny_skia::Renderer>,
{
    fn kind(&self) -> crate::aux::focus::AuxKind {
        B::KIND
    }

    fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
        &mut self.shell
    }
}

impl<B, S> crate::aux::focus::AuxEvents for WidgetWindow<B, S>
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

pub(crate) type SettingsWindowHandle = WidgetWindow<SettingsBinding, SettingsMailbox>;

impl SettingsWindowHandle {
    pub(crate) fn new(
        window: nerust_frontend_host::window::Window,
        binding: SettingsBinding,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
    ) -> Option<Self> {
        let signals = SettingsMailbox {
            should_close: binding.should_close.clone(),
            pending_apply: binding.pending_apply.clone(),
            view_invalidated: binding.view_invalidated.clone(),
        };
        Self::open(window, binding, event_loop, signals, (960.0, 720.0))
    }
}

impl crate::aux::registry::AuxController for SettingsWindowHandle {
    fn as_settings(&mut self) -> Option<&mut dyn crate::aux::registry::SettingsClosePort> {
        Some(self)
    }
}

impl crate::aux::registry::SettingsClosePort for SettingsWindowHandle {
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot> {
        self.signals.take_pending_apply()
    }
}

impl WidgetSignals<SettingsBinding> for SettingsMailbox {
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
            super::bindings::keyboard_key_from_physical(*physical_key).map(Message::CaptureKey)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub(crate) struct HistorySignals {
    should_close: Arc<AtomicBool>,
    clear_requested: Arc<AtomicBool>,
    view_invalidated: AtomicBool,
}

impl WidgetSignals<HistoryBinding> for HistorySignals {
    fn close_flag(&self) -> &Arc<AtomicBool> {
        &self.should_close
    }
    fn refresh_flag(&self) -> &AtomicBool {
        &self.view_invalidated
    }
}

pub(crate) type HistoryWindowHandle = WidgetWindow<HistoryBinding, HistorySignals>;

impl HistoryWindowHandle {
    pub(crate) fn new(
        window: nerust_frontend_host::window::Window,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        language: AppLanguage,
    ) -> Option<Self> {
        let signals = HistorySignals::default();
        let binding = HistoryBinding::new(language, signals.clear_requested.clone());
        Self::open(window, binding, event_loop, signals, (720.0, 360.0))
    }
}

impl crate::aux::registry::AuxController for HistoryWindowHandle {
    fn as_history(&mut self) -> Option<&mut dyn crate::aux::registry::HistorySyncPort> {
        Some(self)
    }
}

impl crate::aux::registry::HistorySyncPort for HistoryWindowHandle {
    fn sync_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
        self.shell.process(
            [HistoryMessage::Sync { lines, language }],
            &self.signals.view_invalidated,
        );
    }

    fn take_history_clear(&mut self) -> bool {
        self.signals.clear_requested.swap(false, Ordering::AcqRel)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Event, Point, keyboard, mouse};
    use std::sync::atomic::AtomicBool;
    use tao::{dpi::PhysicalPosition, event::WindowEvent};

    use crate::aux::native::adapt_event;
    use crate::convert::{FeedContext, feed_domain_event};

    #[test]
    fn widget_signals_keep_close_separate_from_repaint_and_clear() {
        let settings = SettingsMailbox::default();
        settings.close_flag().store(true, Ordering::Release);
        settings.refresh_flag().store(true, Ordering::Release);
        assert!(settings.refresh_flag().swap(false, Ordering::AcqRel));
        assert!(settings.close_flag().load(Ordering::Acquire));

        let history = HistorySignals::default();
        history.close_flag().store(true, Ordering::Release);
        history.clear_requested.store(true, Ordering::Release);
        history.refresh_flag().store(true, Ordering::Release);
        assert!(history.refresh_flag().swap(false, Ordering::AcqRel));
        assert!(history.clear_requested.swap(false, Ordering::AcqRel));
        assert!(history.close_flag().load(Ordering::Acquire));
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
