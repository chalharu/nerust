use std::sync::atomic::AtomicBool;

use iced::{Event, Font, Point, keyboard, mouse};
use iced_winit::core::SmolStr;
use tao::keyboard::ModifiersState as TaoModifiers;

/// Convert Tao modifiers to iced modifiers.
pub(crate) fn tao_modifiers_to_iced(m: TaoModifiers) -> keyboard::Modifiers {
    let mut out = keyboard::Modifiers::empty();
    out.set(keyboard::Modifiers::SHIFT, m.contains(TaoModifiers::SHIFT));
    out.set(keyboard::Modifiers::CTRL, m.contains(TaoModifiers::CONTROL));
    out.set(keyboard::Modifiers::ALT, m.contains(TaoModifiers::ALT));
    out.set(keyboard::Modifiers::LOGO, m.contains(TaoModifiers::SUPER));
    out
}

/// Convert Tao KeyCode to iced key::Code.
pub(crate) fn tao_keycode_to_iced_code(code: tao::keyboard::KeyCode) -> keyboard::key::Code {
    let key = nerust_keyboard::Key::try_from(code).unwrap_or(nerust_keyboard::Key::Backquote);
    key.into()
}

/// Convert Tao Key to iced Key. Named editing keys map explicitly;
/// anything else stays unidentified (physical codes cover shortcuts).
pub(crate) fn tao_key_to_iced_key(key: &tao::keyboard::Key) -> keyboard::Key {
    use iced::keyboard::key::Named;
    match key {
        tao::keyboard::Key::Character(s) => keyboard::Key::Character(SmolStr::new(s)),
        tao::keyboard::Key::Backspace => keyboard::Key::Named(Named::Backspace),
        tao::keyboard::Key::Delete => keyboard::Key::Named(Named::Delete),
        tao::keyboard::Key::Enter => keyboard::Key::Named(Named::Enter),
        tao::keyboard::Key::Escape => keyboard::Key::Named(Named::Escape),
        tao::keyboard::Key::Tab => keyboard::Key::Named(Named::Tab),
        _ => keyboard::Key::Unidentified,
    }
}

/// Default font for settings window.
pub(crate) fn default_font() -> Font {
    #[cfg(target_os = "windows")]
    {
        Font::with_name("Yu Gothic UI")
    }
    #[cfg(not(target_os = "windows"))]
    {
        Font::DEFAULT
    }
}

/// Convert a Tao window event to an iced event, tracking cursor,
/// modifiers, and close requests. Shared by the settings and debugger
/// windows.
pub(crate) fn convert_tao_window_event(
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
        // are not needed for utility windows.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_keycodes_have_mapping() {
        use tao::keyboard::KeyCode as T;
        let codes = [
            T::KeyA,
            T::KeyZ,
            T::Digit0,
            T::Digit9,
            T::ArrowUp,
            T::ArrowDown,
            T::ArrowLeft,
            T::ArrowRight,
            T::Enter,
            T::Escape,
            T::Space,
            T::Tab,
            T::Backspace,
            T::Delete,
            T::Home,
            T::End,
            T::F1,
            T::F12,
            T::ShiftLeft,
            T::ControlLeft,
            T::AltLeft,
            T::SuperLeft,
            T::Numpad0,
            T::Numpad9,
            T::NumpadAdd,
            T::NumpadSubtract,
            T::NumpadMultiply,
            T::NumpadDivide,
            T::CapsLock,
            T::NumLock,
            T::ScrollLock,
            T::Comma,
            T::Period,
            T::Semicolon,
            T::Quote,
            T::Minus,
            T::Equal,
            T::BracketLeft,
            T::BracketRight,
            T::Backslash,
            T::Slash,
            T::IntlBackslash,
        ];
        for code in codes {
            let iced_code = tao_keycode_to_iced_code(code);
            // Backquote is the fallback; no tested key should hit it.
            assert_ne!(
                iced_code as i32,
                keyboard::key::Code::Backquote as i32,
                "KeyCode variant {:?} fell through to fallback",
                code,
            );
        }
    }

    #[test]
    fn character_key_round_trip() {
        let key = tao::keyboard::Key::Character("a");
        let iced = tao_key_to_iced_key(&key);
        assert_eq!(iced, keyboard::Key::Character(SmolStr::new("a")));
    }

    #[test]
    fn empty_string_character_preserved() {
        let key = tao::keyboard::Key::Character("");
        assert_eq!(
            tao_key_to_iced_key(&key),
            keyboard::Key::Character(SmolStr::new(""))
        );
    }

    #[test]
    fn window_events_convert_or_pass_through() {
        use std::sync::atomic::AtomicBool;
        use tao::event::{DeviceId, ElementState, WindowEvent};
        let mut cursor = mouse::Cursor::default();
        let mut modifiers = keyboard::Modifiers::default();
        let should_close = AtomicBool::new(false);
        fn convert(
            event: WindowEvent,
            cursor: &mut mouse::Cursor,
            modifiers: &mut keyboard::Modifiers,
            should_close: &AtomicBool,
        ) -> Option<iced::Event> {
            convert_tao_window_event(event, cursor, 1.0, modifiers, should_close)
        }
        macro_rules! feed {
            ($event:expr) => {
                convert($event, &mut cursor, &mut modifiers, &should_close)
            };
        }
        let device = unsafe { DeviceId::dummy() };
        // Cursor movement tracks the pointer.
        let moved = feed!(WindowEvent::CursorMoved {
            device_id: device,
            position: tao::dpi::PhysicalPosition::new(10.0, 20.0),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(matches!(
            moved,
            Some(iced::Event::Mouse(mouse::Event::CursorMoved { .. }))
        ));
        assert!(matches!(cursor, mouse::Cursor::Available(_)));
        // Leaving the window drops the cursor.
        assert!(feed!(WindowEvent::CursorLeft { device_id: device }).is_none());
        assert!(matches!(cursor, mouse::Cursor::Unavailable));
        // Mouse buttons map left/right/middle, others pass through.
        for button in [
            tao::event::MouseButton::Left,
            tao::event::MouseButton::Right,
            tao::event::MouseButton::Middle,
        ] {
            let click = feed!(WindowEvent::MouseInput {
                device_id: device,
                state: ElementState::Pressed,
                button,
                modifiers: tao::keyboard::ModifiersState::empty(),
            });
            assert!(matches!(
                click,
                Some(iced::Event::Mouse(mouse::Event::ButtonPressed(_)))
            ));
        }
        let other = feed!(WindowEvent::MouseInput {
            device_id: device,
            state: ElementState::Pressed,
            button: tao::event::MouseButton::Other(9),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        assert!(other.is_none());
        // Modifiers update state; close flips the flag.
        let changed = feed!(WindowEvent::ModifiersChanged(
            tao::keyboard::ModifiersState::empty(),
        ));
        assert!(changed.is_none());
        assert!(feed!(WindowEvent::CloseRequested).is_none());
        assert!(should_close.load(std::sync::atomic::Ordering::Acquire));
        // Anything else (resizes and friends) passes through silently.
        assert!(feed!(WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600))).is_none());
    }
}
