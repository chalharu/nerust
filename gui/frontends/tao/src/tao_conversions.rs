use iced::Font;
use iced::keyboard;
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

/// Convert Tao Key to iced Key.
pub(crate) fn tao_key_to_iced_key(key: &tao::keyboard::Key) -> keyboard::Key {
    match key {
        tao::keyboard::Key::Character(s) => keyboard::Key::Character(SmolStr::new(s)),
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
}
