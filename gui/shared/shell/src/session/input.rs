use crate::session::{KeyboardShortcut, NesSession};
use crate::settings::bindings::events::controller::controller_event_for_key;
use crate::settings::bindings::events::shortcut::shortcut_action_for_key;
use nerust_contract_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_nes::codec::{decode_input_state, encode_input_state};
use nerust_input_schema::DigitalInputEvent;

impl NesSession {
    pub fn handle_controller_input(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
        self.apply_current_input_state();
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

        if let Some(controller_input) =
            controller_event_for_key(&self.settings_snapshot.shared, key, pressed)
        {
            self.handle_controller_input(controller_input);
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

    pub fn clear_controller_input(&mut self) {
        self.pressed_keys.clear();
        let _ = self.input.clear_current_frame();
        self.apply_current_input_state();
    }

    fn current_input_frame(&self) -> Option<nerust_input_nes::frame::NesInputFrame> {
        let bytes = self.session.current_input_state().ok()?;
        match decode_input_state(&bytes) {
            Ok(frame) => Some(frame),
            Err(error) => {
                log::warn!("NES input state decode failed: {error}");
                None
            }
        }
    }

    fn apply_current_input_state(&mut self) {
        let bytes = match encode_input_state(self.input.current_frame()) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("NES input state encode failed: {error}");
                return;
            }
        };
        self.session.apply_input_state(bytes);
    }

    pub(super) fn sync_input_from_session(&mut self) {
        if let Some(frame) = self.current_input_frame() {
            self.input.sync_from_frame(frame);
        } else {
            self.input = Default::default();
        }
    }
}
