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
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const RETURN: u16 = 0x0D;
    pub const SPACE: u16 = 0x20;
    pub const ESCAPE: u16 = 0x1B;
    pub const F4: u16 = 0x73;
    pub const F6: u16 = 0x75;
}

/// `WM_COMMAND` menu ids (resource.h).
pub mod menu {
    pub const PAUSE: u32 = 40001;
    pub const RESUME: u32 = 40002;
    pub const RESET: u32 = 40003;
    pub const SAVE_SLOT: u32 = 40010;
    pub const LOAD_SLOT: u32 = 40011;
    pub const DEBUGGER: u32 = 40020;
    pub const FULLSCREEN: u32 = 40030;
}

/// Virtual-key code to shared key. `None` falls through to `DefWindowProc`.
pub fn win_key_to_nerust(vk: u16) -> Option<Key> {
    Some(match vk {
        vk::LEFT => Key::ArrowLeft,
        vk::UP => Key::ArrowUp,
        vk::RIGHT => Key::ArrowRight,
        vk::DOWN => Key::ArrowDown,
        vk::RETURN => Key::Enter,
        vk::SPACE => Key::Space,
        vk::ESCAPE => Key::Escape,
        vk::F4 => Key::F4,
        vk::F6 => Key::F6,
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
        _ => return None,
    })
}

/// Menu id to a complete host event: commands run through the session,
/// chrome actions through shortcuts — the same split the gtk twin uses.
pub fn win_menu_event(id: u32) -> Option<SessionEvent> {
    use menu as m;
    Some(match id {
        m::PAUSE => SessionEvent::Command(SessionCommand::Pause),
        m::RESUME => SessionEvent::Command(SessionCommand::Resume),
        m::RESET => SessionEvent::Command(SessionCommand::Reset),
        m::SAVE_SLOT => SessionEvent::Command(SessionCommand::SaveActiveSlotOrNew),
        m::LOAD_SLOT => SessionEvent::Command(SessionCommand::LoadActiveSlot),
        m::DEBUGGER => SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger),
        m::FULLSCREEN => SessionEvent::Shortcut(KeyboardShortcut::ToggleFullscreen),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_letters_map() {
        assert_eq!(win_key_to_nerust(vk::LEFT), Some(Key::ArrowLeft));
        assert_eq!(win_key_to_nerust(vk::RETURN), Some(Key::Enter));
        assert_eq!(win_key_to_nerust(0x41), Some(Key::KeyA));
        assert_eq!(win_key_to_nerust(0x5A), Some(Key::KeyZ));
        assert_eq!(win_key_to_nerust(0x00), None);
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
