use iced::{Event, Point, keyboard as k, mouse};
use tao::{event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent}, keyboard as tk};
pub(super) fn key(key: &tk::Key<'_>) -> k::Key {
    use k::key::Named;
    if let tk::Key::Character(text) = key { return k::Key::Character((*text).into()); }
    k::Key::Named(match key {
        tk::Key::Space => Named::Space, tk::Key::Enter => Named::Enter, tk::Key::Tab => Named::Tab,
        tk::Key::Backspace => Named::Backspace, tk::Key::Delete => Named::Delete, tk::Key::Escape => Named::Escape,
        tk::Key::ArrowLeft => Named::ArrowLeft, tk::Key::ArrowRight => Named::ArrowRight,
        tk::Key::ArrowUp => Named::ArrowUp, tk::Key::ArrowDown => Named::ArrowDown,
        tk::Key::Home => Named::Home, tk::Key::End => Named::End, tk::Key::PageUp => Named::PageUp,
        tk::Key::PageDown => Named::PageDown, tk::Key::Shift => Named::Shift, tk::Key::Control => Named::Control,
        tk::Key::Alt => Named::Alt, tk::Key::Super => Named::Super, tk::Key::Copy => Named::Copy,
        tk::Key::Cut => Named::Cut, tk::Key::Paste => Named::Paste,
        _ => return k::Key::Unidentified,
    })
}
pub(super) fn convert(event: &WindowEvent<'_>, scale: f64, offset: Point,
    cursor: &mut mouse::Cursor, modifiers: &mut k::Modifiers) -> Option<Event> {
    Some(match event {
        WindowEvent::CursorMoved { position, .. } => {
            let position = Point::new(position.x as f32 / scale as f32 - offset.x, position.y as f32 / scale as f32 - offset.y);
            *cursor = if position.y < 0.0 || position.x < 0.0 { mouse::Cursor::Unavailable }
                else { mouse::Cursor::Available(position) };
            Event::Mouse(mouse::Event::CursorMoved { position })
        }
        WindowEvent::CursorLeft { .. } => { *cursor = mouse::Cursor::Unavailable; Event::Mouse(mouse::Event::CursorLeft) }
        WindowEvent::MouseInput { state, button, .. } => {
            let button = match button { MouseButton::Left => mouse::Button::Left, MouseButton::Right => mouse::Button::Right,
                MouseButton::Middle => mouse::Button::Middle, MouseButton::Other(n) => mouse::Button::Other(*n), _ => return None };
            Event::Mouse(if *state == ElementState::Pressed { mouse::Event::ButtonPressed(button) } else { mouse::Event::ButtonReleased(button) })
        }
        WindowEvent::MouseWheel { delta, .. } => Event::Mouse(mouse::Event::WheelScrolled { delta: match delta {
            MouseScrollDelta::LineDelta(x, y) => mouse::ScrollDelta::Lines { x: *x, y: *y },
            MouseScrollDelta::PixelDelta(p) => mouse::ScrollDelta::Pixels { x: (p.x / scale) as f32, y: (p.y / scale) as f32 },
            _ => return None,
        } }),
        WindowEvent::ModifiersChanged(m) => {
            *modifiers = k::Modifiers::empty(); modifiers.set(k::Modifiers::SHIFT, m.shift_key());
            modifiers.set(k::Modifiers::CTRL, m.control_key()); modifiers.set(k::Modifiers::ALT, m.alt_key());
            modifiers.set(k::Modifiers::LOGO, m.super_key()); Event::Keyboard(k::Event::ModifiersChanged(*modifiers))
        }
        WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => {
            let key = key(&event.key_without_modifiers()); let modified_key = self::key(&event.logical_key);
            let physical_key = k::key::Physical::Unidentified(k::key::NativeCode::Unidentified);
            let location = match event.location { tk::KeyLocation::Left => k::Location::Left,
                tk::KeyLocation::Right => k::Location::Right, tk::KeyLocation::Numpad => k::Location::Numpad, _ => k::Location::Standard };
            Event::Keyboard(if event.state == ElementState::Pressed {
                k::Event::KeyPressed { key, modified_key, physical_key, location, modifiers: *modifiers,
                    // Linux IMContextSimple emits ordinary text via ReceivedImeText too.
                    text: if cfg!(target_os = "linux") { None } else {
                        event.text.filter(|t| !t.chars().any(char::is_control)).map(Into::into) }, repeat: event.repeat }
            } else { k::Event::KeyReleased { key, modified_key, physical_key, location, modifiers: *modifiers } })
        }
        WindowEvent::ReceivedImeText(text) => Event::InputMethod(iced::advanced::input_method::Event::Commit(text.clone())),
        WindowEvent::Focused(focused) => {
            if !focused { *modifiers = k::Modifiers::empty(); }
            Event::Window(if *focused { iced::window::Event::Focused } else { iced::window::Event::Unfocused })
        }
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(deprecated)]
    fn hidpi_and_native_menu_offset_and_commit() {
        let mut cursor = mouse::Cursor::Unavailable; let mut modifiers = k::Modifiers::empty();
        convert(&WindowEvent::CursorMoved { device_id: unsafe { tao::event::DeviceId::dummy() },
            position: tao::dpi::PhysicalPosition::new(80.0, 120.0), modifiers: tk::ModifiersState::empty() },
            2.0, Point::new(0.0, 24.0), &mut cursor, &mut modifiers);
        assert_eq!(cursor.position(), Some(Point::new(40.0, 36.0)));
        assert_eq!(convert(&WindowEvent::ReceivedImeText("日本語".into()), 1.0, Point::ORIGIN, &mut cursor, &mut modifiers),
            Some(Event::InputMethod(iced::advanced::input_method::Event::Commit("日本語".into()))));
    }
    #[test]
    fn limits_are_explicit() {
        let caps = crate::View::input_capabilities();
        assert!(caps.system_clipboard && caps.ime_commit);
        assert!(!caps.ime_preedit && !caps.physical_keys && !caps.os_accessibility);
        assert_eq!(key(&tk::Key::Backspace), k::Key::Named(k::key::Named::Backspace));
    }
}