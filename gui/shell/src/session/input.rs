use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_traits::InputValue;

use crate::{
    session::{KeyboardShortcut, SessionHandle},
    settings::bindings::events::shortcut::shortcut_action_for_key,
};

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
                let slot = binding.attachment.as_str();
                let control = binding.control.as_str();
                if let Some(&field) = self.field_map.get(&(slot, control)) {
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
