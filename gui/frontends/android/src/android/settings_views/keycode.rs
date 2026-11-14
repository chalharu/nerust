//! Android keycode to pad position: the single frontend table.
//!
//! Both the JNI key path and the winit fallback share this. Keycodes
//! 104/105 (L2/R2 key events) stay unmapped: triggers are analog-only and
//! arrive as motion axes.
//!
//! The winit side (`winit_key_to_key`) covers hardware keyboards in
//! gameplay: named USB codes map directly, Android-specific codes reuse
//! the settings capture table below, and anything else (system keys,
//! media remotes) stays `None` so the platform keeps it.

use nerust_input_traits::{Key, StandardButton};
use winit::keyboard::{KeyCode, NativeKeyCode, PhysicalKey};

/// Raw Android keycode to position button. `None` for keyboard keys,
/// triggers (104/105), and unknown codes.
pub(crate) fn android_keycode_to_button(keycode: u32) -> Option<StandardButton> {
    const TABLE: [(u32, StandardButton); 15] = [
        (19, StandardButton::DpadUp),
        (20, StandardButton::DpadDown),
        (21, StandardButton::DpadLeft),
        (22, StandardButton::DpadRight),
        (96, StandardButton::South),
        (97, StandardButton::East),
        (99, StandardButton::West),
        (100, StandardButton::North),
        (102, StandardButton::LeftShoulder),
        (103, StandardButton::RightShoulder),
        (106, StandardButton::LeftStick),
        (107, StandardButton::RightStick),
        (108, StandardButton::Start),
        (109, StandardButton::Back),
        (110, StandardButton::Guide),
    ];
    TABLE
        .binary_search_by_key(&keycode, |(code, _)| *code)
        .ok()
        .map(|index| TABLE[index].1)
}

/// Raw Android keycode to emulated key (settings capture vocabulary).
/// `None` for pad-only buttons, triggers, and system keys, so capture and
/// gameplay agree on what a keyboard can ever claim.
pub(crate) fn android_keycode_to_key(keycode: i32) -> Option<Key> {
    // Sorted for binary search; mirrors the settings capture vocabulary
    // so gameplay and capture never disagree on a code.
    const TABLE: [(i32, Key); 98] = [
        (7, Key::Digit0),
        (8, Key::Digit1),
        (9, Key::Digit2),
        (10, Key::Digit3),
        (11, Key::Digit4),
        (12, Key::Digit5),
        (13, Key::Digit6),
        (14, Key::Digit7),
        (15, Key::Digit8),
        (16, Key::Digit9),
        (19, Key::ArrowUp),
        (20, Key::ArrowDown),
        (21, Key::ArrowLeft),
        (22, Key::ArrowRight),
        (29, Key::KeyA),
        (30, Key::KeyB),
        (31, Key::KeyC),
        (32, Key::KeyD),
        (33, Key::KeyE),
        (34, Key::KeyF),
        (35, Key::KeyG),
        (36, Key::KeyH),
        (37, Key::KeyI),
        (38, Key::KeyJ),
        (39, Key::KeyK),
        (40, Key::KeyL),
        (41, Key::KeyM),
        (42, Key::KeyN),
        (43, Key::KeyO),
        (44, Key::KeyP),
        (45, Key::KeyQ),
        (46, Key::KeyR),
        (47, Key::KeyS),
        (48, Key::KeyT),
        (49, Key::KeyU),
        (50, Key::KeyV),
        (51, Key::KeyW),
        (52, Key::KeyX),
        (53, Key::KeyY),
        (54, Key::KeyZ),
        (55, Key::Comma),
        (56, Key::Period),
        (57, Key::AltLeft),
        (58, Key::AltRight),
        (59, Key::ShiftLeft),
        (60, Key::ShiftRight),
        (61, Key::Tab),
        (62, Key::Space),
        (66, Key::Enter),
        (67, Key::Backspace),
        (68, Key::Backquote),
        (69, Key::Minus),
        (70, Key::Equal),
        (71, Key::BracketLeft),
        (72, Key::BracketRight),
        (73, Key::Backslash),
        (74, Key::Semicolon),
        (75, Key::Quote),
        (76, Key::Slash),
        (92, Key::PageUp),
        (93, Key::PageDown),
        (111, Key::Escape),
        (112, Key::Delete),
        (113, Key::ControlLeft),
        (114, Key::ControlRight),
        (115, Key::CapsLock),
        (122, Key::Home),
        (123, Key::End),
        (124, Key::Insert),
        (131, Key::F1),
        (132, Key::F2),
        (133, Key::F3),
        (134, Key::F4),
        (135, Key::F5),
        (136, Key::F6),
        (137, Key::F7),
        (138, Key::F8),
        (139, Key::F9),
        (140, Key::F10),
        (141, Key::F11),
        (142, Key::F12),
        (144, Key::Numpad0),
        (145, Key::Numpad1),
        (146, Key::Numpad2),
        (147, Key::Numpad3),
        (148, Key::Numpad4),
        (149, Key::Numpad5),
        (150, Key::Numpad6),
        (151, Key::Numpad7),
        (152, Key::Numpad8),
        (153, Key::Numpad9),
        (154, Key::NumpadDivide),
        (155, Key::NumpadMultiply),
        (156, Key::NumpadSubtract),
        (157, Key::NumpadAdd),
        (158, Key::NumpadDecimal),
        (163, Key::NumpadEnter),
        (164, Key::NumpadEqual),
    ];
    TABLE
        .binary_search_by_key(&keycode, |(code, _)| *code)
        .ok()
        .map(|index| TABLE[index].1)
}

/// Winit physical key to emulated key for gameplay. Named USB codes map
/// directly; Android-specific codes reuse the settings capture table so
/// both paths name the same key. Unmapped (system keys, media remotes,
/// unknown natives) stays `None`: the platform keeps those.
pub(crate) fn winit_key_to_key(key: &PhysicalKey) -> Option<Key> {
    match key {
        PhysicalKey::Code(code) => Some(match code {
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
            KeyCode::Comma => Key::Comma,
            KeyCode::Period => Key::Period,
            KeyCode::Minus => Key::Minus,
            KeyCode::Equal => Key::Equal,
            KeyCode::BracketLeft => Key::BracketLeft,
            KeyCode::BracketRight => Key::BracketRight,
            KeyCode::Backslash => Key::Backslash,
            KeyCode::Semicolon => Key::Semicolon,
            KeyCode::Quote => Key::Quote,
            KeyCode::Backquote => Key::Backquote,
            KeyCode::Slash => Key::Slash,
            KeyCode::Tab => Key::Tab,
            KeyCode::Space => Key::Space,
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Escape => Key::Escape,
            KeyCode::Delete => Key::Delete,
            KeyCode::Insert => Key::Insert,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::ArrowUp => Key::ArrowUp,
            KeyCode::ArrowDown => Key::ArrowDown,
            KeyCode::ArrowLeft => Key::ArrowLeft,
            KeyCode::ArrowRight => Key::ArrowRight,
            KeyCode::CapsLock => Key::CapsLock,
            KeyCode::ShiftLeft => Key::ShiftLeft,
            KeyCode::ShiftRight => Key::ShiftRight,
            KeyCode::ControlLeft => Key::ControlLeft,
            KeyCode::ControlRight => Key::ControlRight,
            KeyCode::AltLeft => Key::AltLeft,
            KeyCode::AltRight => Key::AltRight,
            KeyCode::SuperLeft => Key::SuperLeft,
            KeyCode::SuperRight => Key::SuperRight,
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
            KeyCode::NumpadAdd => Key::NumpadAdd,
            KeyCode::NumpadSubtract => Key::NumpadSubtract,
            KeyCode::NumpadMultiply => Key::NumpadMultiply,
            KeyCode::NumpadDivide => Key::NumpadDivide,
            KeyCode::NumpadDecimal => Key::NumpadDecimal,
            KeyCode::NumpadEnter => Key::NumpadEnter,
            KeyCode::NumpadEqual => Key::NumpadEqual,
            _ => return None,
        }),
        PhysicalKey::Unidentified(NativeKeyCode::Android(code)) => {
            i32::try_from(*code).ok().and_then(android_keycode_to_key)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_keycodes_cover_letters_space_and_function_keys() {
        for (keycode, key) in [
            (29, Key::KeyA),
            (54, Key::KeyZ),
            (62, Key::Space),
            (66, Key::Enter),
            (111, Key::Escape),
            (131, Key::F1),
            (142, Key::F12),
            (144, Key::Numpad0),
            (158, Key::NumpadDecimal),
        ] {
            assert_eq!(android_keycode_to_key(keycode), Some(key));
        }
        // Pad buttons, triggers, and system keys never name a keyboard key.
        for keycode in [4, 24, 25, 96, 104, 105, 110] {
            assert_eq!(android_keycode_to_key(keycode), None);
        }
    }

    #[test]
    fn winit_codes_cover_usb_keys_and_reuse_the_android_table() {
        use winit::keyboard::PhysicalKey;
        for (code, key) in [
            (KeyCode::KeyA, Key::KeyA),
            (KeyCode::KeyZ, Key::KeyZ),
            (KeyCode::Digit0, Key::Digit0),
            (KeyCode::Space, Key::Space),
            (KeyCode::Enter, Key::Enter),
            (KeyCode::Escape, Key::Escape),
            (KeyCode::ArrowUp, Key::ArrowUp),
            (KeyCode::F12, Key::F12),
            (KeyCode::NumpadEnter, Key::NumpadEnter),
            (KeyCode::ShiftLeft, Key::ShiftLeft),
            (KeyCode::SuperRight, Key::SuperRight),
        ] {
            assert_eq!(winit_key_to_key(&PhysicalKey::Code(code)), Some(key));
        }
        // Android-specific codes ride the same table as capture.
        assert_eq!(
            winit_key_to_key(&PhysicalKey::Unidentified(NativeKeyCode::Android(29))),
            Some(Key::KeyA)
        );
        assert_eq!(
            winit_key_to_key(&PhysicalKey::Unidentified(NativeKeyCode::Android(96))),
            None,
            "pad buttons never name a keyboard key"
        );
        assert_eq!(
            winit_key_to_key(&PhysicalKey::Unidentified(NativeKeyCode::Unidentified)),
            None
        );
    }

    #[test]
    fn keycodes_cover_face_dpad_and_system_buttons() {
        for (keycode, button) in [
            (19, StandardButton::DpadUp),
            (20, StandardButton::DpadDown),
            (21, StandardButton::DpadLeft),
            (22, StandardButton::DpadRight),
            (96, StandardButton::South),
            (97, StandardButton::East),
            (99, StandardButton::West),
            (100, StandardButton::North),
            (102, StandardButton::LeftShoulder),
            (103, StandardButton::RightShoulder),
            (106, StandardButton::LeftStick),
            (107, StandardButton::RightStick),
            (108, StandardButton::Start),
            (109, StandardButton::Back),
            (110, StandardButton::Guide),
        ] {
            assert_eq!(android_keycode_to_button(keycode), Some(button));
        }
        for keycode in [0, 98, 104, 105, 255] {
            assert_eq!(android_keycode_to_button(keycode), None);
        }
    }
}
