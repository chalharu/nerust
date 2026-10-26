//! Win32 input translation: virtual keys and menu ids become host events.
//!
//! The tables are intentionally small: they prove the translation shape,
//! not the full accelerator table. Production fills in the remaining
//! `WM_KEYDOWN`/`WM_COMMAND` entries following the same two functions.

use nerust_frontend_host::session::SessionEvent;
use nerust_gui_shell::session::{KeyboardShortcut, commands::SessionCommand};
use nerust_keyboard::Key;

/// `WM_KEYDOWN`/`WM_KEYUP` `wParam` virtual-key codes.
pub mod vk {
    pub const BACKSPACE: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const CAPSLOCK: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PAGEUP: u16 = 0x21;
    pub const PAGEDOWN: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
    pub const VOLUME_MUTE: u16 = 0xAD;
    pub const VOLUME_DOWN: u16 = 0xAE;
    pub const VOLUME_UP: u16 = 0xAF;
    pub const MEDIA_NEXT: u16 = 0xB0;
    pub const MEDIA_PREV: u16 = 0xB1;
    pub const MEDIA_STOP: u16 = 0xB2;
    pub const MEDIA_PLAY_PAUSE: u16 = 0xB3;
    pub const SEMICOLON: u16 = 0xBA;
    pub const EQUAL: u16 = 0xBB;
    pub const COMMA: u16 = 0xBC;
    pub const MINUS: u16 = 0xBD;
    pub const PERIOD: u16 = 0xBE;
    pub const SLASH: u16 = 0xBF;
    pub const BACKQUOTE: u16 = 0xC0;
    pub const BRACKET_LEFT: u16 = 0xDB;
    pub const BACKSLASH: u16 = 0xDC;
    pub const BRACKET_RIGHT: u16 = 0xDD;
    pub const QUOTE: u16 = 0xDE;
}

/// `WM_COMMAND` menu ids (resource.h).
pub mod menu {
    pub const FILE_OPEN: u32 = 40000;
    pub const FILE_CLOSE: u32 = 40001;
    pub const FILE_EXIT: u32 = 40002;
    pub const PAUSE: u32 = 40010;
    pub const RESUME: u32 = 40011;
    pub const RESET: u32 = 40012;
    pub const CREATE_SLOT: u32 = 40020;
    pub const SAVE_SLOT: u32 = 40021;
    pub const LOAD_SLOT: u32 = 40022;
    pub const NEXT_SLOT: u32 = 40023;
    pub const PREV_SLOT: u32 = 40024;
    pub const DEBUGGER: u32 = 40030;
    pub const PPU: u32 = 40031;
    pub const FULLSCREEN: u32 = 40032;
    pub const SETTINGS: u32 = 40033;
}

/// Virtual-key code to shared key. Left/right modifiers resolve through
/// the extended `VK_L*`/`VK_R*` codes; the generic `VK_SHIFT`/`VK_CONTROL`/
/// `VK_MENU` fall back to the left variant. `None` falls through to
/// `DefWindowProc`.
pub fn win_key_to_nerust(vk: u16) -> Option<Key> {
    Some(match vk {
        vk::BACKSPACE => Key::Backspace,
        vk::TAB => Key::Tab,
        vk::RETURN => Key::Enter,
        vk::SHIFT => Key::ShiftLeft,
        vk::CONTROL => Key::ControlLeft,
        vk::MENU => Key::AltLeft,
        vk::CAPSLOCK => Key::CapsLock,
        vk::ESCAPE => Key::Escape,
        vk::SPACE => Key::Space,
        vk::PAGEUP => Key::PageUp,
        vk::PAGEDOWN => Key::PageDown,
        vk::END => Key::End,
        vk::HOME => Key::Home,
        vk::LEFT => Key::ArrowLeft,
        vk::UP => Key::ArrowUp,
        vk::RIGHT => Key::ArrowRight,
        vk::DOWN => Key::ArrowDown,
        vk::INSERT => Key::Insert,
        vk::DELETE => Key::Delete,
        vk::LSHIFT => Key::ShiftLeft,
        vk::RSHIFT => Key::ShiftRight,
        vk::LCONTROL => Key::ControlLeft,
        vk::RCONTROL => Key::ControlRight,
        vk::LMENU => Key::AltLeft,
        vk::RMENU => Key::AltRight,
        vk::VOLUME_MUTE => Key::AudioVolumeMute,
        vk::VOLUME_DOWN => Key::AudioVolumeDown,
        vk::VOLUME_UP => Key::AudioVolumeUp,
        vk::MEDIA_NEXT => Key::MediaTrackNext,
        vk::MEDIA_PREV => Key::MediaTrackPrevious,
        vk::MEDIA_STOP => Key::MediaStop,
        vk::MEDIA_PLAY_PAUSE => Key::MediaPlayPause,
        vk::SEMICOLON => Key::Semicolon,
        vk::EQUAL => Key::Equal,
        vk::COMMA => Key::Comma,
        vk::MINUS => Key::Minus,
        vk::PERIOD => Key::Period,
        vk::SLASH => Key::Slash,
        vk::BACKQUOTE => Key::Backquote,
        vk::BRACKET_LEFT => Key::BracketLeft,
        vk::BACKSLASH => Key::Backslash,
        vk::BRACKET_RIGHT => Key::BracketRight,
        vk::QUOTE => Key::Quote,
        0x30..=0x39 => match vk - 0x30 {
            0 => Key::Digit0,
            1 => Key::Digit1,
            2 => Key::Digit2,
            3 => Key::Digit3,
            4 => Key::Digit4,
            5 => Key::Digit5,
            6 => Key::Digit6,
            7 => Key::Digit7,
            8 => Key::Digit8,
            _ => Key::Digit9,
        },
        0x41..=0x5A => match vk - 0x41 {
            0 => Key::KeyA,
            1 => Key::KeyB,
            2 => Key::KeyC,
            3 => Key::KeyD,
            4 => Key::KeyE,
            5 => Key::KeyF,
            6 => Key::KeyG,
            7 => Key::KeyH,
            8 => Key::KeyI,
            9 => Key::KeyJ,
            10 => Key::KeyK,
            11 => Key::KeyL,
            12 => Key::KeyM,
            13 => Key::KeyN,
            14 => Key::KeyO,
            15 => Key::KeyP,
            16 => Key::KeyQ,
            17 => Key::KeyR,
            18 => Key::KeyS,
            19 => Key::KeyT,
            20 => Key::KeyU,
            21 => Key::KeyV,
            22 => Key::KeyW,
            23 => Key::KeyX,
            24 => Key::KeyY,
            _ => Key::KeyZ,
        },
        0x60..=0x69 => match vk - 0x60 {
            0 => Key::Numpad0,
            1 => Key::Numpad1,
            2 => Key::Numpad2,
            3 => Key::Numpad3,
            4 => Key::Numpad4,
            5 => Key::Numpad5,
            6 => Key::Numpad6,
            7 => Key::Numpad7,
            8 => Key::Numpad8,
            _ => Key::Numpad9,
        },
        0x6A => Key::NumpadMultiply,
        0x6B => Key::NumpadAdd,
        0x6C => Key::NumpadComma,
        0x6D => Key::NumpadSubtract,
        0x6E => Key::NumpadDecimal,
        0x6F => Key::NumpadDivide,
        0x70..=0x87 => match vk - 0x70 {
            0 => Key::F1,
            1 => Key::F2,
            2 => Key::F3,
            3 => Key::F4,
            4 => Key::F5,
            5 => Key::F6,
            6 => Key::F7,
            7 => Key::F8,
            8 => Key::F9,
            9 => Key::F10,
            10 => Key::F11,
            11 => Key::F12,
            12 => Key::F13,
            13 => Key::F14,
            14 => Key::F15,
            15 => Key::F16,
            16 => Key::F17,
            17 => Key::F18,
            18 => Key::F19,
            19 => Key::F20,
            20 => Key::F21,
            21 => Key::F22,
            22 => Key::F23,
            _ => Key::F24,
        },
        _ => return None,
    })
}

/// Menu id to a complete host event: commands run through the session,
/// chrome actions through shortcuts — the same split the gtk twin uses.
/// `FILE_OPEN`/`FILE_EXIT` are shell-level: the example handles them
/// around this map (dialog + quit), never as session commands.
pub fn win_menu_event(id: u32) -> Option<SessionEvent> {
    use menu as m;
    Some(match id {
        m::PAUSE => SessionEvent::Command(SessionCommand::Pause),
        m::RESUME => SessionEvent::Command(SessionCommand::Resume),
        m::RESET => SessionEvent::Command(SessionCommand::Reset),
        m::CREATE_SLOT => SessionEvent::Command(SessionCommand::CreateSlot),
        m::SAVE_SLOT => SessionEvent::Command(SessionCommand::SaveActiveSlotOrNew),
        m::LOAD_SLOT => SessionEvent::Command(SessionCommand::LoadActiveSlot),
        m::NEXT_SLOT => SessionEvent::Command(SessionCommand::SelectNextSlot),
        m::PREV_SLOT => SessionEvent::Command(SessionCommand::SelectPreviousSlot),
        m::DEBUGGER => SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger),
        m::FULLSCREEN => SessionEvent::Shortcut(KeyboardShortcut::ToggleFullscreen),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_table_maps_without_gaps() {
        // Letters, digits, numpad, F-keys: every code resolves.
        for vk in 0x41..=0x5A {
            assert!(win_key_to_nerust(vk).is_some(), "{vk:#X}");
        }
        for vk in 0x30..=0x39 {
            assert!(win_key_to_nerust(vk).is_some(), "{vk:#X}");
        }
        for vk in 0x60..=0x6F {
            assert!(win_key_to_nerust(vk).is_some(), "{vk:#X}");
        }
        for vk in 0x70..=0x87 {
            assert!(win_key_to_nerust(vk).is_some(), "{vk:#X}");
        }
        assert_eq!(win_key_to_nerust(0x6A), Some(Key::NumpadMultiply));
        assert_eq!(win_key_to_nerust(0x6B), Some(Key::NumpadAdd));
        assert_eq!(win_key_to_nerust(0x6D), Some(Key::NumpadSubtract));
        assert_eq!(win_key_to_nerust(0x6E), Some(Key::NumpadDecimal));
        assert_eq!(win_key_to_nerust(0x6F), Some(Key::NumpadDivide));
        assert_eq!(win_key_to_nerust(0x73), Some(Key::F4));
        assert_eq!(win_key_to_nerust(vk::LSHIFT), Some(Key::ShiftLeft));
        assert_eq!(win_key_to_nerust(vk::RSHIFT), Some(Key::ShiftRight));
        assert_eq!(
            win_key_to_nerust(vk::VOLUME_MUTE),
            Some(Key::AudioVolumeMute)
        );
        assert_eq!(win_key_to_nerust(vk::SEMICOLON), Some(Key::Semicolon));
    }

    #[test]
    fn arrows_and_letters_map() {
        assert_eq!(win_key_to_nerust(vk::LEFT), Some(Key::ArrowLeft));
        assert_eq!(win_key_to_nerust(vk::RETURN), Some(Key::Enter));
        assert_eq!(win_key_to_nerust(0x41), Some(Key::KeyA));
        assert_eq!(win_key_to_nerust(0x5A), Some(Key::KeyZ));
        assert_eq!(win_key_to_nerust(0x00), None);
    }

    #[test]
    fn all_menu_ids_become_host_events() {
        for id in [
            menu::FILE_OPEN,
            menu::FILE_CLOSE,
            menu::FILE_EXIT,
            menu::PAUSE,
            menu::RESUME,
            menu::RESET,
            menu::CREATE_SLOT,
            menu::SAVE_SLOT,
            menu::LOAD_SLOT,
            menu::NEXT_SLOT,
            menu::PREV_SLOT,
            menu::DEBUGGER,
            menu::FULLSCREEN,
        ] {
            // FILE_* are shell-level (dialog/quit around the map);
            // every other id must resolve to a host event.
            match id {
                menu::FILE_OPEN | menu::FILE_CLOSE | menu::FILE_EXIT => {
                    assert!(win_menu_event(id).is_none())
                }
                _ => assert!(win_menu_event(id).is_some(), "{id}"),
            }
        }
    }

    #[test]
    fn menu_ids_become_host_events() {
        assert!(matches!(
            win_menu_event(menu::PAUSE),
            Some(SessionEvent::Command(SessionCommand::Pause))
        ));
        assert!(matches!(
            win_menu_event(menu::DEBUGGER),
            Some(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger))
        ));
        assert!(win_menu_event(99999).is_none());
    }
}
