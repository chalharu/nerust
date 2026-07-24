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

/// Map a controller profile + port group index to a device kind string.
///
/// Delegates to `ControllerProfile::device_kind_for_group()`.
/// Systems with special port group naming override the trait method.
pub fn device_kind(profile: &dyn ControllerProfile, group_index: usize) -> &'static str {
    profile.device_kind_for_group(group_index)
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
                    let dk = device_kind(profile, gi);
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

/// Clear other occupied slots in the same multi-port set.
///
/// When a controller occupies a port group with >1 ports,
/// other ports in that set must be cleared to avoid conflicts.
pub fn clear_multi_port_conflicts(
    slot: AttachmentId,
    profile: &dyn ControllerProfile,
    assignments: &mut [(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
) {
    for ps in profile.port_sets() {
        if ps.ports.len() <= 1 || !ps.ports.contains(&slot) {
            continue;
        }
        for &port in ps.ports {
            if port != slot
                && let Some(other) = assignments.iter_mut().find(|(s, _)| *s == port)
            {
                other.1 = None;
            }
        }
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

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use nerust_input_traits::{AttachmentId, ControlInfo, ControllerProfile, PortSet, ProfileId};

    use super::*;

    /// Single-port mock: "test.standard" profile with one port.
    #[derive(Debug)]
    struct MockSinglePort;
    impl ControllerProfile for MockSinglePort {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.standard")
        }
        fn label(&self) -> &'static str {
            "Test Standard"
        }
        fn port_sets(&self) -> &[PortSet] {
            static P: &[AttachmentId] = &[AttachmentId::new("test.slot")];
            static S: &[PortSet] = &[PortSet { ports: P }];
            S
        }
        fn port_groups(&self) -> &[&[ControlInfo]] {
            &[]
        }
    }

    /// Multi-port mock: "test.multi" profile with P1/P2 ports in one set.
    #[derive(Debug)]
    struct MockMultiPort;
    impl ControllerProfile for MockMultiPort {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.multi")
        }
        fn label(&self) -> &'static str {
            "Test Multi"
        }
        fn port_sets(&self) -> &[PortSet] {
            static P: &[AttachmentId] =
                &[AttachmentId::new("test.p1"), AttachmentId::new("test.p2")];
            static S: &[PortSet] = &[PortSet { ports: P }];
            S
        }
        fn port_groups(&self) -> &[&[ControlInfo]] {
            &[]
        }
    }

    #[test]
    fn clear_multi_port_does_nothing_for_single_port() {
        let profile = MockSinglePort;
        let slot = AttachmentId::new("test.slot");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> =
            vec![(slot, Some(Rc::new(MockSinglePort)))];
        clear_multi_port_conflicts(slot, &profile, &mut assignments);
        assert!(assignments[0].1.is_some());
    }

    #[test]
    fn clear_multi_port_clears_other_ports() {
        let profile = MockMultiPort;
        let p1 = AttachmentId::new("test.p1");
        let p2 = AttachmentId::new("test.p2");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = vec![
            (p1, Some(Rc::new(MockMultiPort))),
            (p2, Some(Rc::new(MockMultiPort))),
        ];
        clear_multi_port_conflicts(p1, &profile, &mut assignments);
        assert!(assignments[0].1.is_some(), "P1 should stay assigned");
        assert!(assignments[1].1.is_none(), "P2 should be cleared");
    }

    #[test]
    fn clear_multi_port_does_not_clear_unrelated() {
        let profile = MockMultiPort;
        let p1 = AttachmentId::new("test.p1");
        let p2 = AttachmentId::new("test.p2");
        let other = AttachmentId::new("test.other");
        let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = vec![
            (other, Some(Rc::new(MockSinglePort))),
            (p1, Some(Rc::new(MockMultiPort))),
            (p2, Some(Rc::new(MockMultiPort))),
        ];
        clear_multi_port_conflicts(p1, &profile, &mut assignments);
        assert!(assignments[0].1.is_some(), "Unrelated port unchanged");
        assert!(assignments[1].1.is_some(), "P1 stays");
        assert!(assignments[2].1.is_none(), "P2 cleared");
    }

    #[test]
    fn device_kind_delegates_to_profile_method() {
        let profile = MockSinglePort;
        let kind = device_kind(&profile, 0);
        assert_eq!(kind, "test.standard");
    }
}
