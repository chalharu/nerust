use std::sync::{Arc, Mutex, atomic::AtomicBool};

use iced_winit::program;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use tao::event_loop::EventLoopWindowTarget;

use super::program::{Message, SettingsAppProgram, find_duplicate_system_id};
use crate::aux::bridge::lock_recovered;
use crate::aux::shell::{AuxShell, AuxSurface};

/// Settings dialog: shared [`AuxShell`] host plus pending-apply staging.
/// Matches the debugger/PPU shape.
pub(crate) struct SettingsWindowHandle {
    shell: AuxShell<SettingsAppProgram>,
    view_invalidated: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
}

impl SettingsWindowHandle {
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
    ) -> Option<Self> {
        if let Some(duplicate) = find_duplicate_system_id(&registry) {
            log::error!("refusing settings window: duplicate SystemId '{duplicate}'");
            return None;
        }
        let should_close = Arc::new(AtomicBool::new(false));
        let pending_apply = Arc::new(Mutex::new(None));
        let view_invalidated = Arc::new(AtomicBool::new(false));

        let program = SettingsAppProgram {
            snapshot,
            registry,
            audio_registry,
            should_close: Arc::clone(&should_close),
            pending_apply: Arc::clone(&pending_apply),
            view_invalidated: Arc::clone(&view_invalidated),
        };
        let (instance, _task) = program::Instance::new(program);
        let surface = AuxSurface::open(
            event_loop,
            "Preferences",
            (960.0, 720.0),
            None,
            None,
            "settings window",
            Arc::clone(&should_close),
        )
        .map_err(|error| {
            // Typed scaffold failure: surface the tagged native detail here.
            log::error!("{error}");
            error
        })
        .ok()?;
        // Birth geometry is a placeholder on some compositors; record
        // it to tell "born 0x0, never resized" apart from later traffic.
        log::debug!(
            "settings create: physical={}x{} scale={} bounds={:?}",
            surface.viewport_physical.0,
            surface.viewport_physical.1,
            surface.scale_factor,
            surface.bounds,
        );
        let shell = AuxShell::assemble(surface, instance);
        Some(Self {
            shell,
            view_invalidated,
            pending_apply,
        })
    }

    pub(crate) fn take_pending_apply(&mut self) -> Option<SettingsSnapshot> {
        lock_recovered(&self.pending_apply).take()
    }
}

impl crate::aux::focus::AuxShellRef for SettingsWindowHandle {
    fn kind(&self) -> crate::aux::focus::AuxKind {
        crate::aux::focus::AuxKind::Settings
    }

    fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
        &mut self.shell
    }
}

impl crate::aux::registry::DebugDrainPort for SettingsWindowHandle {}
impl crate::aux::registry::LiveImagesPort for SettingsWindowHandle {}
impl crate::aux::registry::HistorySyncPort for SettingsWindowHandle {}
impl crate::aux::registry::BridgeSharePort for SettingsWindowHandle {}
impl crate::aux::registry::AuxController for SettingsWindowHandle {}

impl crate::aux::registry::SettingsClosePort for SettingsWindowHandle {
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot> {
        self.take_pending_apply()
    }
}

impl crate::aux::focus::AuxEvents for SettingsWindowHandle {
    fn handle(&mut self, mapped: iced::Event) -> bool {
        let capture = if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
        {
            crate::aux::settings::program::keyboard_key_from_physical(*physical_key)
                .map(Message::CaptureKey)
        } else {
            None
        };
        self.shell
            .pump_finish(mapped, capture, &self.view_invalidated)
    }
}
#[cfg(test)]
mod tests {
    use iced::{Event, Point, keyboard, mouse};
    use tao::{dpi::PhysicalPosition, event::WindowEvent};

    use super::*;
    use crate::aux::native::adapt_event;
    use crate::convert::feed_domain_event;

    fn initial_cursor() -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(0.0, 0.0))
    }

    fn feed(
        event: WindowEvent,
        cursor: &mut mouse::Cursor,
        modifiers: &mut keyboard::Modifiers,
        should_close: &AtomicBool,
    ) -> Option<Event> {
        feed_domain_event(adapt_event(&event), cursor, modifiers, 1.0, should_close)
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
