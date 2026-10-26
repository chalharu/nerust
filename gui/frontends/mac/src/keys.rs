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
    pub const LEFT: u16 = 123;
    pub const RIGHT: u16 = 124;
    pub const DOWN: u16 = 125;
    pub const UP: u16 = 126;
    pub const RETURN: u16 = 36;
    pub const SPACE: u16 = 49;
    pub const ESCAPE: u16 = 53;
    pub const F4: u16 = 118;
    pub const F6: u16 = 97;
    pub const A: u16 = 0;
}

/// `NSMenuItem` action selectors.
pub mod selector {
    pub const PAUSE: &str = "pauseEmulation:";
    pub const RESUME: &str = "resumeEmulation:";
    pub const RESET: &str = "resetEmulation:";
    pub const SAVE_SLOT: &str = "saveStateSlot:";
    pub const LOAD_SLOT: &str = "loadStateSlot:";
    pub const DEBUGGER: &str = "toggleDebugger:";
    pub const FULLSCREEN: &str = "toggleFullscreen:";
}

/// Key code to shared key. `None` falls through the responder chain.
pub fn mac_key_to_nerust(code: u16) -> Option<Key> {
    Some(match code {
        keycode::LEFT => Key::ArrowLeft,
        keycode::UP => Key::ArrowUp,
        keycode::RIGHT => Key::ArrowRight,
        keycode::DOWN => Key::ArrowDown,
        keycode::RETURN => Key::Enter,
        keycode::SPACE => Key::Space,
        keycode::ESCAPE => Key::Escape,
        keycode::F4 => Key::F4,
        keycode::F6 => Key::F6,
        keycode::A => Key::KeyA,
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
        s::SAVE_SLOT => SessionEvent::Command(SessionCommand::SaveActiveSlotOrNew),
        s::LOAD_SLOT => SessionEvent::Command(SessionCommand::LoadActiveSlot),
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
        assert_eq!(mac_key_to_nerust(999), None);
    }

    #[test]
    fn selectors_become_host_events() {
        assert!(matches!(
            mac_menu_event(selector::PAUSE),
            Some(SessionEvent::Command(SessionCommand::Pause))
        ));
        assert!(matches!(
            mac_menu_event(selector::DEBUGGER),
            Some(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger))
        ));
        assert!(mac_menu_event("unknown:").is_none());
    }
}
