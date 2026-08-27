use std::{collections::HashMap, hash::Hash};

use nerust_core_traits::touch::{TouchControl, TouchControlRole, TouchOverlayModel};
use nerust_gui_settings::input::{KeyboardBinding, ShortcutAction};
use nerust_input_traits::{
    AbstractKey, AttachmentId, DigitalControlId, DigitalInputEvent, InputAssignments, InputValue,
};
use nerust_keyboard::Key;
use nerust_settings_core::factory::settings_view;

use crate::{
    session::{KeyboardShortcut, SessionError, SessionHandle},
    settings::bindings::events::shortcut::shortcut_action_for_key,
};

pub use nerust_settings_core::input::{build_topology, clear_multi_port_conflicts, device_kind};

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

impl SessionHandle {
    pub fn touch_overlay_model(&self, revision: u64) -> TouchOverlayModel {
        let mut controls = Vec::new();
        for (attachment_id, profile) in &self.current_assignments.slots {
            let Some(profile) = profile else {
                continue;
            };
            let group = profile
                .port_sets()
                .iter()
                .position(|set| set.ports.contains(attachment_id))
                .and_then(|index| profile.port_groups().get(index));
            let Some(group) = group else {
                continue;
            };
            for info in *group {
                let role = match info.abstract_key {
                    Some(AbstractKey::DpadUp) => TouchControlRole::DpadUp,
                    Some(AbstractKey::DpadDown) => TouchControlRole::DpadDown,
                    Some(AbstractKey::DpadLeft) => TouchControlRole::DpadLeft,
                    Some(AbstractKey::DpadRight) => TouchControlRole::DpadRight,
                    Some(AbstractKey::Button1) => TouchControlRole::FaceButton1,
                    Some(AbstractKey::Button2) => TouchControlRole::FaceButton2,
                    Some(AbstractKey::Start) => TouchControlRole::Start,
                    Some(AbstractKey::Select) => TouchControlRole::Select,
                    _ => continue,
                };
                controls.push(TouchControl {
                    attachment_id: *attachment_id,
                    control_id: info.id,
                    role,
                    label: info.label.to_string(),
                });
            }
        }
        TouchOverlayModel { revision, controls }
    }

    /// Reassign controllers and rebuild the core.
    pub fn reassign_controllers(
        &mut self,
        assignments: &InputAssignments,
    ) -> Result<(), crate::session::SessionError> {
        let factory = self.active_factory().ok_or(SessionError::NoCore)?;
        let system_id = factory.system_id();
        let view = settings_view(&self.settings_snapshot, system_id.as_ref());
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
        self.key_field_map.clear();
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

    use nerust_gui_settings::input::PersistedControlId;
    use nerust_input_traits::{
        AbstractKey, AttachmentId, ControlInfo, ControlKind, ControllerProfile, PortSet, ProfileId,
        SlotInfo,
    };

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

    #[derive(Debug)]
    struct MockTopologyProfile;

    impl ControllerProfile for MockTopologyProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.topology")
        }

        fn label(&self) -> &'static str {
            "Topology Pad"
        }

        fn port_sets(&self) -> &[PortSet] {
            static PORTS: &[AttachmentId] = &[AttachmentId::new("test.topology.slot")];
            static SETS: &[PortSet] = &[PortSet { ports: PORTS }];
            SETS
        }

        fn port_groups(&self) -> &[&[ControlInfo]] {
            static CONTROLS: &[ControlInfo] = &[ControlInfo {
                id: DigitalControlId::new("test.topology.a"),
                label: "A",
                kind: ControlKind::Digital,
                abstract_key: Some(AbstractKey::Button1),
            }];
            static GROUPS: &[&[ControlInfo]] = &[CONTROLS];
            GROUPS
        }

        fn device_kind_for_group(&self, _group_index: usize) -> &'static str {
            "test.topology.pad"
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
    fn build_topology_returns_empty_model_without_controllers() {
        let topology = build_topology(&[], &[]);

        assert!(topology.ports.is_empty());
        assert!(topology.devices.is_empty());
    }

    #[test]
    fn build_topology_describes_assigned_controller() {
        let slot = AttachmentId::new("test.topology.slot");
        let profile: Rc<dyn ControllerProfile> = Rc::new(MockTopologyProfile);
        let topology = build_topology(
            &[(slot, Some(profile))],
            &[SlotInfo {
                id: slot,
                label: "Topology Port",
            }],
        );

        assert_eq!(topology.ports.len(), 1);
        assert_eq!(topology.ports[0].label, "Topology Port");
        assert_eq!(topology.devices.len(), 1);
        assert_eq!(topology.devices[0].kind.as_str(), "test.topology.pad");
        assert_eq!(topology.devices[0].controls.len(), 1);
    }

    #[test]
    fn rebuild_input_map_maps_matching_keyboard_binding() {
        let attachment = AttachmentId::new("test.topology.slot");
        let control = DigitalControlId::new("test.topology.a");
        let field_map = HashMap::from([((attachment, control), 7)]);
        let bindings = [KeyboardBinding::new(
            attachment.as_str(),
            PersistedControlId::digital(control.as_str()),
            Key::KeyA,
        )];
        let mut key_map = HashMap::from([(Key::KeyB, 2)]);

        rebuild_input_map(&field_map, &bindings, &mut key_map);

        assert_eq!(key_map, HashMap::from([(Key::KeyA, 7)]));
    }

    #[test]
    fn touch_overlay_uses_assigned_profile_controls() {
        let mut session = crate::session::test_util::test_session();
        let slot = AttachmentId::new("test.topology.slot");
        session.current_assignments.slots = vec![(slot, Some(Rc::new(MockTopologyProfile)))];

        let overlay = session.touch_overlay_model(42);

        assert_eq!(overlay.revision, 42);
        assert_eq!(overlay.controls.len(), 1);
        assert_eq!(overlay.controls[0].attachment_id, slot);
        assert_eq!(overlay.controls[0].control_id.as_str(), "test.topology.a");
        assert_eq!(overlay.controls[0].role, TouchControlRole::FaceButton1);
        assert_eq!(overlay.controls[0].label, "A");
    }

    #[test]
    fn device_kind_delegates_to_profile_method() {
        let profile = MockSinglePort;
        let kind = device_kind(&profile, 0);
        assert_eq!(kind, "test.standard");
    }
}
