use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_traits::InputValue;

use crate::{
    session::{KeyboardShortcut, SessionHandle},
    settings::bindings::events::shortcut::shortcut_action_for_key,
};

/// Resolve a (attachment_id, control_id) string pair to a field index for NES.
fn field_index_for_nes(attachment: &str, control: &str) -> Option<usize> {
    let player = if attachment.ends_with("player1") {
        0
    } else if attachment.ends_with("player2") {
        1
    } else {
        return None;
    };
    let base = player * 8;
    match control {
        "nes.control.a" => Some(base + 0),
        "nes.control.b" => Some(base + 1),
        "nes.control.select" => Some(base + 2),
        "nes.control.start" => Some(base + 3),
        "nes.control.up" => Some(base + 4),
        "nes.control.down" => Some(base + 5),
        "nes.control.left" => Some(base + 6),
        "nes.control.right" => Some(base + 7),
        _ if attachment.ends_with("player2") && control == "famicom.microphone" => Some(16),
        _ => None,
    }
}

impl SessionHandle {
    pub fn handle_keyboard_key(
        &mut self,
        key: KeyboardKey,
        pressed: bool,
    ) -> Option<KeyboardShortcut> {
        let first_press = if pressed {
            self.pressed_keys.insert(key)
        } else {
            self.pressed_keys.remove(&key);
            false
        };

        let system_id = self.factory.system_id();
        let profile = self
            .settings_snapshot
            .shared
            .input
            .systems
            .get(&system_id)
            .and_then(|s| s.implicit_keyboard_profile());
        if let Some(profile) = profile {
            if let Some(binding) = profile.bindings.iter().find(|b| b.key == key) {
                if let Some(field) = field_index_for_nes(
                    binding.attachment.as_str(),
                    binding.control.as_str(),
                ) {
                    let _ = self.gui_input.write_buf.set(
                        field,
                        InputValue::Digital(pressed),
                    );
                }
            }
        }

        if first_press {
            return shortcut_action_for_key(&self.settings_snapshot.shared, key).map(|action| {
                if matches!(action, ShortcutAction::ToggleFullscreen) {
                    KeyboardShortcut::ToggleFullscreen
                } else {
                    KeyboardShortcut::Session(action)
                }
            });
        }
        None
    }

    pub fn clear_input(&mut self) {
        self.pressed_keys.clear();
        self.gui_input.clear();
    }
}
