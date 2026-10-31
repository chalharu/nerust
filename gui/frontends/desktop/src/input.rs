use panechord::KeyCode;
use wakui_core::Key;

pub(crate) fn session_action(
    action: nerust_settings_core::model::input::ShortcutAction,
) -> Option<nerust_gui_application::SessionAction> {
    use nerust_gui_application::SessionAction;
    use nerust_settings_core::model::input::ShortcutAction;
    Some(match action {
        ShortcutAction::Reset => SessionAction::Reset,
        ShortcutAction::SaveActiveSlot => SessionAction::SaveState,
        ShortcutAction::LoadActiveSlot => SessionAction::LoadState,
        ShortcutAction::SelectNextSlot => SessionAction::NextSlot,
        ShortcutAction::SelectPreviousSlot => SessionAction::PreviousSlot,
        ShortcutAction::TogglePause
        | ShortcutAction::ToggleFullscreen
        | ShortcutAction::OpenDebugger => return None,
    })
}

pub(crate) fn key(code: &KeyCode) -> Option<Key> {
    macro_rules! keys {
        ($($name:ident),+ $(,)?) => {
            match code.as_str() { $(stringify!($name) => Some(Key::$name),)+ _ => None }
        };
    }
    keys!(
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Enter,
        Escape,
        Space,
        Tab,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        F13,
        F14,
        F15,
        F16,
        F17,
        F18,
        F19,
        F20,
        F21,
        F22,
        F23,
        F24,
        F25,
        F26,
        F27,
        F28,
        F29,
        F30,
        F31,
        F32,
        F33,
        F34,
        F35,
        Backspace,
        Delete,
        Home,
        End,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        Backquote,
        Comma,
        Period,
        Slash,
        Semicolon,
        Quote,
        BracketLeft,
        BracketRight,
        Backslash,
        Minus,
        Equal,
        IntlRo,
        IntlYen,
        Hiragana,
        Katakana,
        KanaMode,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadDecimal,
        NumpadAdd,
        NumpadSubtract,
        NumpadMultiply,
        NumpadDivide,
        NumpadEnter,
        NumpadEqual,
        Insert,
        IntlBackslash,
        PageUp,
        PageDown,
        CapsLock,
        ScrollLock,
        Pause,
        PrintScreen,
        Convert,
        NonConvert,
        Plus,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_controller_shortcut_and_international_keys() {
        assert_eq!(key(&KeyCode::new("Enter")), Some(Key::Enter));
        assert_eq!(key(&KeyCode::new("ArrowRight")), Some(Key::ArrowRight));
        assert_eq!(key(&KeyCode::new("F11")), Some(Key::F11));
        assert_eq!(key(&KeyCode::new("IntlYen")), Some(Key::IntlYen));
        assert_eq!(key(&KeyCode::new("BrowserBack")), None);
    }
}
