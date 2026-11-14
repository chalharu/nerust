//! Android keycode to pad position: the single frontend table.
//!
//! Both the JNI key path and the winit fallback share this. Keycodes
//! 104/105 (L2/R2 key events) stay unmapped: triggers are analog-only and
//! arrive as motion axes. Winit translates D-pad keys to arrows, so the
//! arrow fallback lives here too; callers only see `StandardButton`.

use winit::keyboard::{KeyCode, NativeKeyCode, PhysicalKey};

use nerust_input_traits::StandardButton;

/// Raw Android keycode to position button. `None` for keyboard keys,
/// triggers (104/105), and unknown codes.
pub(crate) fn android_keycode_to_button(keycode: i32) -> Option<StandardButton> {
    const TABLE: [(i32, StandardButton); 15] = [
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

/// Winit-delivered pad keys: raw codes plus winit's arrow translation.
/// `None` for keyboard keys.
pub(crate) fn winit_pad_button(key: &PhysicalKey) -> Option<StandardButton> {
    match key {
        PhysicalKey::Unidentified(NativeKeyCode::Android(code)) => {
            android_keycode_to_button(*code)
        }
        PhysicalKey::Code(KeyCode::ArrowUp) => Some(StandardButton::DpadUp),
        PhysicalKey::Code(KeyCode::ArrowDown) => Some(StandardButton::DpadDown),
        PhysicalKey::Code(KeyCode::ArrowLeft) => Some(StandardButton::DpadLeft),
        PhysicalKey::Code(KeyCode::ArrowRight) => Some(StandardButton::DpadRight),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
