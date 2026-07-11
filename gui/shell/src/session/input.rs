use std::collections::HashMap;

use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_traits::{DigitalInputEvent, InputAssignments, InputValue};

use crate::{
    session::{KeyboardShortcut, SessionHandle},
    settings::{bindings::events::shortcut::shortcut_action_for_key, factory::settings_view},
};

/// Normalize a binding ID (e.g. "nes.attachment.player1" or "nes.control.a")
/// to the short form used in field_map keys.
pub fn normalize_id(id: &str) -> &str {
    id.trim_start_matches("nes.attachment.")
        .trim_start_matches("nes.control.")
        .trim_start_matches("famicom.")
}

/// Map slot name to attachment ID string.
pub fn attachment_id(slot: &str) -> &'static str {
    match slot {
        "player1" => "nes.attachment.player1",
        "player2" => "nes.attachment.player2",
        _ => "unknown",
    }
}

/// Map control short name to control ID string.
pub fn control_id(id: &str) -> &'static str {
    match id {
        "a" => "nes.control.a",
        "b" => "nes.control.b",
        "select" => "nes.control.select",
        "start" => "nes.control.start",
        "up" => "nes.control.up",
        "down" => "nes.control.down",
        "left" => "nes.control.left",
        "right" => "nes.control.right",
        "microphone" => "famicom.microphone",
        _ => "unknown",
    }
}

/// Map controller kind + port group index to device kind string.
pub fn device_kind(ctrl_id: &'static str, group_index: usize) -> &'static str {
    match (ctrl_id, group_index) {
        ("nes.famicom", 1) => "nes.famicom_p2",
        _ => ctrl_id,
    }
}

impl SessionHandle {
    /// Reassign controllers and rebuild the core.
    pub fn reassign_controllers(
        &mut self,
        assignments: &InputAssignments,
    ) -> Result<(), crate::session::SessionError> {
        let system_id = self.factory.system_id();
        let view = settings_view(&self.settings_snapshot, &system_id);
        let speaker =
            crate::settings::build_speaker(&self.audio_registry, &self.settings_snapshot.local);
        let parts =
            self.factory
                .create_core_and_adapter_with_assignments(&view, speaker, assignments)?;
        let (rebuilt_core, gui_input, field_map) = crate::emu_core::EmuCore::from_parts(parts);
        let was_paused = self.emu_core.metrics().paused;
        if let Some(loaded_media) = self.loaded_media.clone() {
            rebuilt_core.load(&loaded_media.media, Vec::new())?;
            if !was_paused {
                rebuilt_core.resume()?;
            }
        }
        self.emu_core = rebuilt_core;
        self.gui_input = gui_input;
        self.field_map = field_map;
        self.current_assignments = assignments.clone();
        Ok(())
    }

    /// Called by touch overlay (Android) with a pre-resolved DigitalInputEvent.
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) {
        let slot = normalize_id(event.attachment.as_str());
        let control = normalize_id(event.control.as_str());
        if let Some(&field) = self.field_map.get(&(slot, control)) {
            let _ = self
                .gui_input
                .state
                .set(field, InputValue::Digital(event.is_pressed()));
        }
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

        let system_id = self.factory.system_id();
        let profile = self
            .settings_snapshot
            .shared
            .input
            .systems
            .get(&system_id)
            .and_then(|s| s.implicit_keyboard_profile());

        let matched_profile = self
            .field_map
            .iter()
            .filter_map(|((slot, control), field)| {
                let attachment_id = attachment_id(slot);
                let control_id = control_id(control);
                let binding = profile?.bindings.iter().find(|b| {
                    b.attachment.as_str() == attachment_id && b.control.as_str() == control_id
                })?;
                Some((binding.key, *field))
            })
            .collect::<HashMap<_, _>>();

        if let Some(&field) = matched_profile.get(&key) {
            let _ = self
                .gui_input
                .state
                .set(field, InputValue::Digital(pressed));
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
