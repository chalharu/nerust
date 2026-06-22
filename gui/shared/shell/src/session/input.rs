use crate::session::{KeyboardShortcut, SessionHandle};
use crate::settings::bindings::events::controller::controller_event_for_key;
use crate::settings::bindings::events::shortcut::shortcut_action_for_key;
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_contract_input::DigitalInputEvent;

impl SessionHandle {
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) {
        self.input_adapter.apply_event(event);
    }

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

        let system_id = self.descriptor.system_id;
        if let Some(controller_input) = controller_event_for_key(
            &self.settings_snapshot.shared,
            system_id,
            key,
            pressed,
            |attachment, control, pressed| {
                self.input_adapter
                    .decode_persisted_input(attachment, control, pressed)
            },
        ) {
            self.apply_input_event(controller_input);
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
        self.input_adapter.clear();
    }
}
