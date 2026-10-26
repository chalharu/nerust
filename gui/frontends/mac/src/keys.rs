//! AppKit input translation: key codes and menu selectors become host events.
//!
//! Same shape as the win twin, different tables: macOS virtual key codes
//! and `NSMenuItem` selector strings. Cmd-shortcuts arrive pre-resolved
//! by the responder chain; this layer only names the host event.

use nerust_frontend_host::session::SessionEvent;
use nerust_gui_shell::session::{KeyboardShortcut, commands::SessionCommand};
use nerust_keyboard::Key;

/// `NSEvent keyCode` values (kVK_*).
pub mod keycode {
    pub const A: u16 = 0;
    pub const S: u16 = 1;
    pub const D: u16 = 2;
    pub const F: u16 = 3;
    pub const H: u16 = 4;
    pub const G: u16 = 5;
    pub const Z: u16 = 6;
    pub const X: u16 = 7;
    pub const C: u16 = 8;
    pub const V: u16 = 9;
    pub const B: u16 = 11;
    pub const Q: u16 = 12;
    pub const W: u16 = 13;
    pub const E: u16 = 14;
    pub const R: u16 = 15;
    pub const Y: u16 = 16;
    pub const T: u16 = 17;
    pub const DIGIT1: u16 = 18;
    pub const DIGIT2: u16 = 19;
    pub const DIGIT3: u16 = 20;
    pub const DIGIT4: u16 = 21;
    pub const DIGIT6: u16 = 22;
    pub const DIGIT5: u16 = 23;
    pub const DIGIT9: u16 = 25;
    pub const DIGIT7: u16 = 26;
    pub const DIGIT8: u16 = 28;
    pub const DIGIT0: u16 = 29;
    pub const O: u16 = 31;
    pub const U: u16 = 32;
    pub const I: u16 = 34;
    pub const P: u16 = 35;
    pub const RETURN: u16 = 36;
    pub const J: u16 = 38;
    pub const K: u16 = 40;
    pub const L: u16 = 37;
    pub const M: u16 = 46;
    pub const N: u16 = 45;
    pub const TAB: u16 = 48;
    pub const SPACE: u16 = 49;
    pub const DELETE: u16 = 51;
    pub const ESCAPE: u16 = 53;
    pub const SHIFT: u16 = 56;
    pub const CONTROL: u16 = 59;
    pub const F5: u16 = 96;
    pub const F6: u16 = 97;
    pub const F7: u16 = 98;
    pub const F8: u16 = 100;
    pub const F9: u16 = 101;
    pub const F11: u16 = 103;
    pub const F4: u16 = 118;
    pub const LEFT: u16 = 123;
    pub const RIGHT: u16 = 124;
    pub const DOWN: u16 = 125;
    pub const UP: u16 = 126;
}

/// `NSMenuItem` action selectors.
pub mod selector {
    pub const PAUSE: &str = "pauseEmulation:";
    pub const RESUME: &str = "resumeEmulation:";
    pub const RESET: &str = "resetEmulation:";
    pub const CREATE_SLOT: &str = "createSaveSlot:";
    pub const SAVE_SLOT: &str = "saveStateSlot:";
    pub const LOAD_SLOT: &str = "loadStateSlot:";
    pub const NEXT_SLOT: &str = "nextSlot:";
    pub const PREV_SLOT: &str = "prevSlot:";
    pub const DEBUGGER: &str = "toggleDebugger:";
    pub const FULLSCREEN: &str = "toggleFullscreen:";
}

/// Key code to shared key. `None` falls through to AppKit beep.
pub fn mac_key_to_nerust(code: u16) -> Option<Key> {
    Some(match code {
        keycode::LEFT => Key::ArrowLeft,
        keycode::UP => Key::ArrowUp,
        keycode::RIGHT => Key::ArrowRight,
        keycode::DOWN => Key::ArrowDown,
        keycode::RETURN => Key::Enter,
        keycode::TAB => Key::Tab,
        keycode::DELETE => Key::Backspace,
        keycode::SPACE => Key::Space,
        keycode::ESCAPE => Key::Escape,
        keycode::SHIFT => Key::ShiftLeft,
        keycode::CONTROL => Key::ControlLeft,
        keycode::F4 => Key::F4,
        keycode::F5 => Key::F5,
        keycode::F6 => Key::F6,
        keycode::F7 => Key::F7,
        keycode::F8 => Key::F8,
        keycode::F9 => Key::F9,
        keycode::F11 => Key::F11,
        keycode::A => Key::KeyA,
        keycode::B => Key::KeyB,
        keycode::C => Key::KeyC,
        keycode::D => Key::KeyD,
        keycode::E => Key::KeyE,
        keycode::F => Key::KeyF,
        keycode::G => Key::KeyG,
        keycode::H => Key::KeyH,
        keycode::I => Key::KeyI,
        keycode::J => Key::KeyJ,
        keycode::K => Key::KeyK,
        keycode::L => Key::KeyL,
        keycode::M => Key::KeyM,
        keycode::N => Key::KeyN,
        keycode::O => Key::KeyO,
        keycode::P => Key::KeyP,
        keycode::Q => Key::KeyQ,
        keycode::R => Key::KeyR,
        keycode::S => Key::KeyS,
        keycode::T => Key::KeyT,
        keycode::U => Key::KeyU,
        keycode::V => Key::KeyV,
        keycode::W => Key::KeyW,
        keycode::X => Key::KeyX,
        keycode::Y => Key::KeyY,
        keycode::Z => Key::KeyZ,
        keycode::DIGIT0 => Key::Digit0,
        keycode::DIGIT1 => Key::Digit1,
        keycode::DIGIT2 => Key::Digit2,
        keycode::DIGIT3 => Key::Digit3,
        keycode::DIGIT4 => Key::Digit4,
        keycode::DIGIT5 => Key::Digit5,
        keycode::DIGIT6 => Key::Digit6,
        keycode::DIGIT7 => Key::Digit7,
        keycode::DIGIT8 => Key::Digit8,
        keycode::DIGIT9 => Key::Digit9,
        _ => return None,
    })
}

/// Menu selector to a complete host event: commands run through the
/// session, chrome actions through shortcuts.
pub fn mac_menu_event(selector: &str) -> Option<SessionEvent> {
    use crate::keys::selector as s;
    Some(match selector {
        s::PAUSE => SessionEvent::Command(SessionCommand::Pause),
        s::RESUME => SessionEvent::Command(SessionCommand::Resume),
        s::RESET => SessionEvent::Command(SessionCommand::Reset),
        s::CREATE_SLOT => SessionEvent::Command(SessionCommand::CreateSlot),
        s::SAVE_SLOT => SessionEvent::Command(SessionCommand::SaveActiveSlotOrNew),
        s::LOAD_SLOT => SessionEvent::Command(SessionCommand::LoadActiveSlot),
        s::NEXT_SLOT => SessionEvent::Command(SessionCommand::SelectNextSlot),
        s::PREV_SLOT => SessionEvent::Command(SessionCommand::SelectPreviousSlot),
        s::DEBUGGER => SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger),
        s::FULLSCREEN => SessionEvent::Shortcut(KeyboardShortcut::ToggleFullscreen),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_return_map() {
        assert_eq!(mac_key_to_nerust(keycode::LEFT), Some(Key::ArrowLeft));
        assert_eq!(mac_key_to_nerust(keycode::RETURN), Some(Key::Enter));
        assert_eq!(mac_key_to_nerust(keycode::A), Some(Key::KeyA));
        assert_eq!(mac_key_to_nerust(keycode::DIGIT0), Some(Key::Digit0));
        assert_eq!(mac_key_to_nerust(keycode::F5), Some(Key::F5));
        assert_eq!(mac_key_to_nerust(keycode::F11), Some(Key::F11));
        assert_eq!(mac_key_to_nerust(999), None);
    }

    #[test]
    fn selectors_become_host_events() {
        assert!(matches!(
            mac_menu_event(selector::PAUSE),
            Some(SessionEvent::Command(SessionCommand::Pause))
        ));
        assert!(matches!(
            mac_menu_event(selector::CREATE_SLOT),
            Some(SessionEvent::Command(SessionCommand::CreateSlot))
        ));
        assert!(matches!(
            mac_menu_event(selector::NEXT_SLOT),
            Some(SessionEvent::Command(SessionCommand::SelectNextSlot))
        ));
        assert!(matches!(
            mac_menu_event(selector::DEBUGGER),
            Some(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger))
        ));
        assert!(mac_menu_event("unknown:").is_none());
    }
}
