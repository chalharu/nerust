use iced::{Font, keyboard};
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
