use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    rc::Rc,
};

use nerust_gui_settings::input::{KeyboardBinding, ShortcutAction};
use nerust_input_traits::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, ControllerProfile, DeviceDescriptor,
    DeviceKindId, DigitalControlDescriptor, DigitalControlId, DigitalInputEvent, InputAssignments,
    InputTopologyDescriptor, InputValue, PortDescriptor, PortId, SlotInfo,
};
use nerust_keyboard::Key;

use crate::{
    session::{KeyboardShortcut, SessionError, SessionHandle},
    settings::{bindings::events::shortcut::shortcut_action_for_key, factory::settings_view},
};

/// Abstraction over a binding type (keyboard, gamepad, etc.) for building
/// a source-key → field-index map from an InputAssignments field_map.
trait InputBinding {
    type Id: Copy + Eq + Hash;
    fn matches(&self, attachment: &AttachmentId, control: &DigitalControlId) -> bool;
    fn source_id(&self) -> Self::Id;
}

impl InputBinding for KeyboardBinding {
    type Id = Key;
    fn matches(&self, attachment: &AttachmentId, control: &DigitalControlId) -> bool {
        self.attachment == *attachment && self.control == *control
    }
    fn source_id(&self) -> Self::Id {
        self.key
    }
}

/// Generic rebuild: iterate field_map, find matching bindings, populate target map.
fn rebuild_input_map<B: InputBinding>(
    field_map: &HashMap<(AttachmentId, DigitalControlId), usize>,
    bindings: &[B],
    target: &mut HashMap<B::Id, usize>,
) {
    target.clear();
    for ((attachment, control), &field) in field_map {
        if let Some(binding) = bindings.iter().find(|b| b.matches(attachment, control)) {
            target.insert(binding.source_id(), field);
        }
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
    assignments: &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
    slots: &[SlotInfo],
) -> InputTopologyDescriptor {
    let slot_label = |att: AttachmentId| -> &'static str {
        slots
            .iter()
            .find(|s| s.id == att)
            .map(|s| s.label)
            .unwrap_or("")
    };
    let mut ports = Vec::new();
    let mut seen_devices = HashSet::<(&str, usize)>::new();
    let mut devices = Vec::new();

    for (slot_att, ctrl_opt) in assignments {
        let profile = match ctrl_opt {
            Some(p) => p.as_ref(),
            None => continue,
        };
        let ctrl_id = profile.profile_id().as_str();
        for ps in profile.port_sets() {
            if ps.ports.contains(slot_att) {
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
                                        id: ci.id,
                                        label: ci.label,
                                        description: ci.label,
                                    })
                                })
                                .collect(),
                        });
                    }
                    if !ports.iter().any(|p: &PortDescriptor| p.id == port) {
                        let label = slot_label(port);
                        ports.push(PortDescriptor {
                            id: PortId::new(port.as_str()),
                            label,
                            attachments: vec![AttachmentSlotDescriptor {
                                id: port,
                                label,
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
        let factory = self.active_factory().ok_or(SessionError::NoCore)?;
        let system_id = factory.system_id();
        let view = settings_view(&self.settings_snapshot, &system_id);
        let speaker =
            crate::settings::build_speaker(&self.audio_registry, &self.settings_snapshot.local);
        let parts =
            factory.create_core_and_adapter_with_assignments(&view, speaker, assignments)?;
        let (rebuilt_core, gui_input, field_map) = crate::emu_core::EmuCore::from_parts(parts);
        let was_paused = self
            .emu_core
            .as_ref()
            .map(|c| c.metrics())
            .unwrap_or_default()
            .paused;
        if let Some(loaded_media) = self.loaded_media.clone() {
            rebuilt_core.load(&loaded_media.media, None)?;
            if !was_paused {
                rebuilt_core.resume()?;
            }
        }
        self.emu_core = Some(rebuilt_core);
        self.gui_input = Some(gui_input);
        self.field_map = field_map;
        self.current_assignments = assignments.clone();
        self.rebuild_key_field_map();
        Ok(())
    }

    /// Called by touch overlay (Android) with a pre-resolved DigitalInputEvent.
    pub fn apply_input_event(&mut self, event: DigitalInputEvent) {
        if let Some(&field) = self.field_map.get(&(event.attachment, event.control))
            && let Some(ref mut gui_input) = self.gui_input
        {
            let _ = gui_input
                .state
                .set(field, InputValue::Digital(event.is_pressed()));
        }
    }

    pub fn handle_keyboard_key(&mut self, key: Key, pressed: bool) -> Option<KeyboardShortcut> {
        let first_press = if pressed {
            self.pressed_keys.insert(key)
        } else {
            self.pressed_keys.remove(&key);
            false
        };

        if let Some(&field) = self.key_field_map.get(&key)
            && let Some(ref mut gui_input) = self.gui_input
        {
            let _ = gui_input.state.set(field, InputValue::Digital(pressed));
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
        if let Some(ref mut gui_input) = self.gui_input {
            gui_input.clear();
        }
    }

    pub fn rebuild_key_field_map(&mut self) {
        let Some(factory) = self.active_factory() else {
            return;
        };
        let system_id = factory.system_id();
        let Some(profile) = self
            .settings_snapshot
            .shared
            .input
            .systems
            .get(&system_id)
            .and_then(|s| s.implicit_keyboard_profile())
        else {
            return;
        };
        rebuild_input_map(&self.field_map, &profile.bindings, &mut self.key_field_map);
    }
}
