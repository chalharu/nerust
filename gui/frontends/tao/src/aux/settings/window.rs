use std::sync::{Arc, Mutex, atomic::AtomicBool};

use iced_winit::program;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use tao::{event_loop::EventLoopWindowTarget, keyboard::ModifiersState as TaoModifiers};

use super::program::{Message, SettingsAppProgram, find_duplicate_system_id};
use crate::aux::bridge::lock_recovered;
use crate::aux::shell::{AuxShell, PumpOutcome, WindowScaffold};
use crate::convert::*;

/// Settings dialog: the shared [`AuxShell`] host plus the
/// settings-only pieces (pending-apply staging and the external
/// view-invalidation flag shared with the program). Matches the
/// debugger/PPU shape so the three aux windows cannot drift apart.
pub(crate) struct SettingsWindowHandle {
    pub(crate) shell: AuxShell<SettingsAppProgram>,
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
        )?;
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

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        let mut messages = Vec::new();

        if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
            && let Some(key) =
                crate::aux::settings::program::keyboard_key_from_physical(*physical_key)
        {
            messages.push(Message::CaptureKey(key));
        }

        let Some(ui) = self.shell.ui_state.ui_mut() else {
            log::warn!("aux window ui unavailable after panicked relayout");
            return;
        };
        let (ui_outcome, _) = ui.update(
            &[mapped],
            self.shell.cursor,
            &mut self.shell.renderer.backend,
            &mut self.shell.clipboard,
            &mut messages,
        );
        let outcome = crate::aux::shell::pump_outcome(&ui_outcome);
        match outcome {
            PumpOutcome::Rebuild => {
                self.view_invalidated
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            PumpOutcome::Repaint => {
                self.shell.window.request_redraw();
            }
            PumpOutcome::Idle => {}
        }

        if !messages.is_empty() || matches!(outcome, PumpOutcome::Rebuild) {
            let bounds = self.shell.logical_bounds();
            let window_id = self.shell.window_id;
            self.shell.ui_state.process_messages(
                messages,
                window_id,
                bounds,
                &mut self.shell.renderer.backend,
                &self.view_invalidated,
            );
        }
    }

    pub(crate) fn render(&mut self) {
        self.shell.render("settings");
    }

    pub(crate) fn take_pending_apply(&mut self) -> Option<SettingsSnapshot> {
        lock_recovered(&self.pending_apply).take()
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: TaoModifiers) {
        self.shell.set_modifiers(modifiers);
    }

    pub(crate) fn update_modifiers_from_tao_event(&mut self, event: &tao::event::WindowEvent) {
        if let tao::event::WindowEvent::ModifiersChanged(state) = event {
            self.set_modifiers(*state);
        }
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        if let Some(iced_event) = convert_tao_window_event(
            event,
            &mut self.shell.cursor,
            self.shell.scale_factor,
            &mut self.shell.modifiers,
            &self.shell.should_close,
        ) {
            self.handle_event(iced_event);
        }
    }
}

impl crate::aux::shell::AuxWindow for SettingsWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.update_modifiers_from_tao_event(&event);
        self.handle_tao_event(event);
        self.shell.window.request_redraw();
    }
}
#[cfg(test)]
mod tests {
    use iced::{Event, Point, keyboard, mouse};
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
