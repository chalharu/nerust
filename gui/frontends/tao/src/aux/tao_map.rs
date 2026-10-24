//! Single tao mapping leaf: codes, buttons, modifiers (tao to domain only).
//! Iced conversion lives in `crate::convert` (domain to iced).

use super::domain::{AuxButton, AuxModifiers, ModifierFlag};
use nerust_keyboard::Key;
use tao::keyboard::{KeyCode, ModifiersState as TaoModifiers};

/// Convert Tao modifiers to domain modifiers.
pub(crate) fn tao_modifiers_to_domain(m: TaoModifiers) -> AuxModifiers {
    let mut out = AuxModifiers::empty();
    if m.contains(TaoModifiers::SHIFT) {
        out = out.with_flag(ModifierFlag::Shift);
    }
    if m.contains(TaoModifiers::CONTROL) {
        out = out.with_flag(ModifierFlag::Control);
    }
    if m.contains(TaoModifiers::ALT) {
        out = out.with_flag(ModifierFlag::Alt);
    }
    if m.contains(TaoModifiers::SUPER) {
        out = out.with_flag(ModifierFlag::Logo);
    }
    out
}
/// Convert a Tao physical key code to the shared [`Key`]: `None` for unhandled codes.
pub(crate) fn keycode_to_key(code: tao::keyboard::KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::KeyA,
        KeyCode::KeyB => Key::KeyB,
        KeyCode::KeyC => Key::KeyC,
        KeyCode::KeyD => Key::KeyD,
        KeyCode::KeyE => Key::KeyE,
        KeyCode::KeyF => Key::KeyF,
        KeyCode::KeyG => Key::KeyG,
        KeyCode::KeyH => Key::KeyH,
        KeyCode::KeyI => Key::KeyI,
        KeyCode::KeyJ => Key::KeyJ,
        KeyCode::KeyK => Key::KeyK,
        KeyCode::KeyL => Key::KeyL,
        KeyCode::KeyM => Key::KeyM,
        KeyCode::KeyN => Key::KeyN,
        KeyCode::KeyO => Key::KeyO,
        KeyCode::KeyP => Key::KeyP,
        KeyCode::KeyQ => Key::KeyQ,
        KeyCode::KeyR => Key::KeyR,
        KeyCode::KeyS => Key::KeyS,
        KeyCode::KeyT => Key::KeyT,
        KeyCode::KeyU => Key::KeyU,
        KeyCode::KeyV => Key::KeyV,
        KeyCode::KeyW => Key::KeyW,
        KeyCode::KeyX => Key::KeyX,
        KeyCode::KeyY => Key::KeyY,
        KeyCode::KeyZ => Key::KeyZ,
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::Enter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::Backquote => Key::Backquote,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        KeyCode::Slash => Key::Slash,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Quote => Key::Quote,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::Minus => Key::Minus,
        KeyCode::Equal => Key::Equal,
        KeyCode::IntlRo => Key::IntlRo,
        KeyCode::IntlYen => Key::IntlYen,
        KeyCode::Hiragana => Key::Hiragana,
        KeyCode::Katakana => Key::Katakana,
        KeyCode::KanaMode => Key::KanaMode,
        KeyCode::Numpad0 => Key::Numpad0,
        KeyCode::Numpad1 => Key::Numpad1,
        KeyCode::Numpad2 => Key::Numpad2,
        KeyCode::Numpad3 => Key::Numpad3,
        KeyCode::Numpad4 => Key::Numpad4,
        KeyCode::Numpad5 => Key::Numpad5,
        KeyCode::Numpad6 => Key::Numpad6,
        KeyCode::Numpad7 => Key::Numpad7,
        KeyCode::Numpad8 => Key::Numpad8,
        KeyCode::Numpad9 => Key::Numpad9,
        KeyCode::NumpadDecimal => Key::NumpadDecimal,
        KeyCode::NumpadAdd => Key::NumpadAdd,
        KeyCode::NumpadSubtract => Key::NumpadSubtract,
        KeyCode::NumpadMultiply => Key::NumpadMultiply,
        KeyCode::NumpadDivide => Key::NumpadDivide,
        KeyCode::NumpadEnter => Key::NumpadEnter,
        KeyCode::Insert => Key::Insert,
        KeyCode::IntlBackslash => Key::IntlBackslash,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::Pause => Key::Pause,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::Convert => Key::Convert,
        KeyCode::NonConvert => Key::NonConvert,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::F25 => Key::F25,
        KeyCode::F26 => Key::F26,
        KeyCode::F27 => Key::F27,
        KeyCode::F28 => Key::F28,
        KeyCode::F29 => Key::F29,
        KeyCode::F30 => Key::F30,
        KeyCode::F31 => Key::F31,
        KeyCode::F32 => Key::F32,
        KeyCode::F33 => Key::F33,
        KeyCode::F34 => Key::F34,
        KeyCode::F35 => Key::F35,
        KeyCode::Plus => Key::Plus,
        KeyCode::NumpadEqual => Key::NumpadEqual,
        KeyCode::NumpadComma => Key::NumpadComma,
        KeyCode::NumpadBackspace => Key::NumpadBackspace,
        KeyCode::NumpadClear => Key::NumpadClear,
        KeyCode::NumpadClearEntry => Key::NumpadClearEntry,
        KeyCode::NumpadHash => Key::NumpadHash,
        KeyCode::NumpadMemoryAdd => Key::NumpadMemoryAdd,
        KeyCode::NumpadMemoryClear => Key::NumpadMemoryClear,
        KeyCode::NumpadMemoryRecall => Key::NumpadMemoryRecall,
        KeyCode::NumpadMemoryStore => Key::NumpadMemoryStore,
        KeyCode::NumpadMemorySubtract => Key::NumpadMemorySubtract,
        KeyCode::NumpadParenLeft => Key::NumpadParenLeft,
        KeyCode::NumpadParenRight => Key::NumpadParenRight,
        KeyCode::NumpadStar => Key::NumpadStar,
        KeyCode::SuperLeft => Key::SuperLeft,
        KeyCode::SuperRight => Key::SuperRight,
        KeyCode::AudioVolumeUp => Key::AudioVolumeUp,
        KeyCode::AudioVolumeDown => Key::AudioVolumeDown,
        KeyCode::AudioVolumeMute => Key::AudioVolumeMute,
        KeyCode::MediaPlayPause => Key::MediaPlayPause,
        KeyCode::MediaStop => Key::MediaStop,
        KeyCode::MediaTrackNext => Key::MediaTrackNext,
        KeyCode::MediaTrackPrevious => Key::MediaTrackPrevious,
        KeyCode::MediaSelect => Key::MediaSelect,
        KeyCode::BrowserBack => Key::BrowserBack,
        KeyCode::BrowserFavorites => Key::BrowserFavorites,
        KeyCode::BrowserForward => Key::BrowserForward,
        KeyCode::BrowserHome => Key::BrowserHome,
        KeyCode::BrowserRefresh => Key::BrowserRefresh,
        KeyCode::BrowserSearch => Key::BrowserSearch,
        KeyCode::BrowserStop => Key::BrowserStop,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::ContextMenu => Key::ContextMenu,
        KeyCode::Power => Key::Power,
        KeyCode::Sleep => Key::Sleep,
        KeyCode::WakeUp => Key::WakeUp,
        KeyCode::Eject => Key::Eject,
        KeyCode::Copy => Key::Copy,
        KeyCode::Cut => Key::Cut,
        KeyCode::Paste => Key::Paste,
        KeyCode::Undo => Key::Undo,
        KeyCode::Find => Key::Find,
        KeyCode::Help => Key::Help,
        KeyCode::Open => Key::Open,
        KeyCode::Select => Key::Select,
        KeyCode::Again => Key::Again,
        KeyCode::Props => Key::Props,
        KeyCode::Abort => Key::Abort,
        KeyCode::Resume => Key::Resume,
        KeyCode::Suspend => Key::Suspend,
        KeyCode::Fn => Key::Fn,
        KeyCode::FnLock => Key::FnLock,
        KeyCode::Hyper => Key::Hyper,
        KeyCode::Turbo => Key::Turbo,
        KeyCode::Lang1 => Key::Lang1,
        KeyCode::Lang2 => Key::Lang2,
        KeyCode::Lang3 => Key::Lang3,
        KeyCode::Lang4 => Key::Lang4,
        KeyCode::Lang5 => Key::Lang5,
        KeyCode::LaunchApp1 => Key::LaunchApp1,
        KeyCode::LaunchApp2 => Key::LaunchApp2,
        KeyCode::LaunchMail => Key::LaunchMail,
        _ => return None,
    })
}
/// Map a native mouse button: unmapped buttons stay `None`.
pub(crate) fn tao_button_to_aux(button: tao::event::MouseButton) -> Option<AuxButton> {
    Some(match button {
        tao::event::MouseButton::Left => AuxButton::Left,
        tao::event::MouseButton::Right => AuxButton::Right,
        tao::event::MouseButton::Middle => AuxButton::Middle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao::keyboard::ModifiersState as M;

    #[test]
    fn modifiers_map_each_flag_to_domain() {
        assert_eq!(tao_modifiers_to_domain(M::empty()), AuxModifiers::empty());
        assert!(tao_modifiers_to_domain(M::SHIFT).shift());
        assert!(tao_modifiers_to_domain(M::CONTROL).ctrl());
        assert!(tao_modifiers_to_domain(M::ALT).alt());
        assert!(tao_modifiers_to_domain(M::SUPER).logo());
        let all = tao_modifiers_to_domain(M::SHIFT | M::CONTROL | M::ALT | M::SUPER);
        assert!(all.shift() && all.ctrl() && all.alt() && all.logo());
    }

    #[test]
    fn mapped_buttons_cross_but_others_drop() {
        use tao::event::MouseButton;
        assert_eq!(tao_button_to_aux(MouseButton::Left), Some(AuxButton::Left));
        assert_eq!(
            tao_button_to_aux(MouseButton::Right),
            Some(AuxButton::Right)
        );
        assert_eq!(
            tao_button_to_aux(MouseButton::Middle),
            Some(AuxButton::Middle)
        );
        assert_eq!(tao_button_to_aux(MouseButton::Other(5)), None);
    }

    #[test]
    fn unmapped_codes_stay_unmapped() {
        let code = tao::keyboard::KeyCode::Unidentified(tao::keyboard::NativeKeyCode::Unidentified);
        assert!(keycode_to_key(code).is_none());
    }
}
