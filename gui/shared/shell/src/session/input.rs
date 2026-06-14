use crate::session::{KeyboardShortcut, SessionHandle};
use crate::settings::bindings::events::controller::controller_event_for_key;
use crate::settings::bindings::events::shortcut::shortcut_action_for_key;
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_schema::DigitalInputEvent;

impl SessionHandle {
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) -> Result<(), String> {
        self.input_adapter.apply_event(event);
        self.apply_current_input_state()
    }

    pub fn handle_keyboard_key(
        &mut self,
        key: KeyboardKey,
        pressed: bool,
    ) -> Result<Option<KeyboardShortcut>, String> {
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
                    .digital_event_from_persisted(attachment, control, pressed)
            },
        ) {
            self.apply_input_event(controller_input)?;
        }

        if first_press {
            return Ok(
                shortcut_action_for_key(&self.settings_snapshot.shared, key).map(|action| {
                    if matches!(action, ShortcutAction::ToggleFullscreen) {
                        KeyboardShortcut::ToggleFullscreen
                    } else {
                        KeyboardShortcut::Session(action)
                    }
                }),
            );
        }
        Ok(None)
    }

    pub fn clear_input(&mut self) -> Result<(), String> {
        self.pressed_keys.clear();
        self.input_adapter.clear();
        self.apply_current_input_state()
    }

    pub(super) fn apply_current_input_state(&mut self) -> Result<(), String> {
        let bytes = self.input_adapter.runtime_state_bytes()?;
        self.runtime.apply_input_state(bytes)
    }

    pub(super) fn sync_input_from_runtime(&mut self) {
        match self.runtime.current_input_state() {
            Ok(bytes) => {
                if let Err(error) = self.input_adapter.sync_from_runtime_state(&bytes) {
                    log::warn!("runtime input sync failed: {error}");
                    self.input_adapter.clear();
                }
            }
            Err(error) => {
                log::warn!("runtime input state read failed: {error}");
                self.input_adapter.clear();
            }
        }
    }
}
