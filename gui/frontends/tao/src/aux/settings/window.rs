use std::sync::{Arc, Mutex, atomic::AtomicBool};

use iced_winit::program;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use tao::event_loop::EventLoopWindowTarget;

use super::program::{Message, SettingsAppProgram, find_duplicate_system_id};
use crate::aux::bridge::lock_recovered;
use crate::aux::shell::{AuxShell, PumpOutcome, WindowScaffold};

/// Settings dialog: the shared [`AuxShell`] host plus the
/// settings-only pieces (pending-apply staging and the external
/// view-invalidation flag shared with the program). Matches the
/// debugger/PPU shape so the three aux windows cannot drift apart.
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
        let scaffold = WindowScaffold::open(
            event_loop,
            "Preferences",
            (960.0, 720.0),
            None,
            None,
            "settings window",
        )
        .map_err(|error| {
            // Typed scaffold failures carry the window tag plus the
            // native detail; surface them here since the opener seam
            // is Option-based.
            log::error!("{error}");
            error
        })
        .ok()?;
        // Birth geometry is a placeholder on some compositors; record
        // it to tell "born 0x0, never resized" apart from later traffic.
        log::debug!(
            "settings create: physical={}x{} scale={} bounds={:?}",
            scaffold.viewport_physical.0,
            scaffold.viewport_physical.1,
            scaffold.scale_factor,
            scaffold.bounds,
        );
        let shell = AuxShell::assemble(scaffold, instance, should_close);
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

impl crate::aux::focus::WindowHandler for SettingsWindowHandle {
    type Program = SettingsAppProgram;
    const LOG_TAG: &str = "settings";

    fn shell(&self) -> &AuxShell<Self::Program> {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut AuxShell<Self::Program> {
        &mut self.shell
    }

    fn handle_event(&mut self, mapped: iced::Event) -> bool {
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
        let (mut messages, outcome) = self.shell.pump_frame(mapped);
        messages.extend(capture);
        if matches!(outcome, PumpOutcome::Rebuild) {
            self.view_invalidated
                .store(true, std::sync::atomic::Ordering::Release);
        }

        let repainted = matches!(outcome, PumpOutcome::Repaint);
        let rebuilt = if !messages.is_empty() || matches!(outcome, PumpOutcome::Rebuild) {
            self.shell.process(
                messages,
                crate::aux::shell::Dirty::settings(&self.view_invalidated),
            )
        } else {
            false
        };
        repainted || rebuilt
    }
}
#[cfg(test)]
mod tests {
    use iced::{Event, Point, keyboard, mouse};
    use tao::{dpi::PhysicalPosition, event::WindowEvent};

    use super::*;
    use crate::convert::convert_tao_window_event;

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
