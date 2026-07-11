use std::collections::HashSet;
use std::rc::Rc;

use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_input_traits::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, ControllerProfile,
    DeviceDescriptor, DeviceKindId, DigitalControlDescriptor, DigitalControlId,
    DigitalInputEvent, InputAssignments, InputTopologyDescriptor, InputValue, PortDescriptor,
    PortId,
};

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

/// Build an InputTopologyDescriptor from slot→controller assignments.
pub fn build_topology(
    assignments: &[(String, Option<Rc<dyn ControllerProfile>>)],
) -> InputTopologyDescriptor {
    let mut ports = Vec::new();
    let mut seen_devices = HashSet::<(&str, usize)>::new();
    let mut devices = Vec::new();

    for (slot_id, ctrl_opt) in assignments {
        let profile = match ctrl_opt {
            Some(p) => p.as_ref(),
            None => continue,
        };
        let ctrl_id = profile.id();
        for ps in profile.port_sets() {
            if ps.ports.iter().any(|&p| p == slot_id) {
                for (gi, &port) in ps.ports.iter().enumerate() {
                    let dk = device_kind(ctrl_id, gi);
                    if seen_devices.insert((ctrl_id, gi)) {
                        let controls = profile.port_groups()[gi];
                        devices.push(DeviceDescriptor {
                            kind: DeviceKindId::new(dk),
                            label: profile.label(),
                            controls: controls
                                .iter()
                                .map(|ci| {
                                    ControlDescriptor::Digital(DigitalControlDescriptor {
                                        id: DigitalControlId::new(control_id(ci.id)),
                                        label: ci.label,
                                        description: ci.label,
                                    })
                                })
                                .collect(),
                        });
                    }
                    let full = attachment_id(port);
                    if !ports.iter().any(|p: &PortDescriptor| p.id.as_str() == full) {
                        ports.push(PortDescriptor {
                            id: PortId::new(full),
                            label: port,
                            attachments: vec![AttachmentSlotDescriptor {
                                id: AttachmentId::new(full),
                                label: port,
                                device: DeviceKindId::new(dk),
                                supported_devices: vec![DeviceKindId::new(dk)],
                            }],
                        });
                    }
                }
            }
        }
    }
    if ports.is_empty() {
        InputTopologyDescriptor {
            ports: Vec::new(),
            devices: Vec::new(),
        }
    } else {
        InputTopologyDescriptor { ports, devices }
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
        self.rebuild_key_field_map();
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

        if let Some(&field) = self.key_field_map.get(&key) {
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
