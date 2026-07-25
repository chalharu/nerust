pub mod descriptors;

use std::collections::BTreeMap;

use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::shared::DesktopSharedSettings;
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

/// Find key binding conflicts within a single system.
///
/// Only the implicit keyboard profile of the given `system` is
/// checked. Shortcut conflicts across the same key are also detected.
pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
) -> BTreeMap<Key, Vec<String>> {
    let mut by_key = BTreeMap::<Key, Vec<String>>::new();

    if let Some(profile) = settings
        .input
        .systems
        .get(system)
        .and_then(|s| s.implicit_keyboard_profile())
    {
        for descriptor in descriptors::keyboard_binding_descriptors(topology, system) {
            if let Some(binding) = profile.bindings.iter().find(|binding| {
                binding.attachment.as_str() == descriptor.attachment.as_str()
                    && binding.control.as_str() == descriptor.control.as_str()
            }) {
                by_key.entry(binding.key).or_default().push(format!(
                    "{} {}",
                    descriptor.attachment_label, descriptor.control_label
                ));
            }
        }
    }

    for descriptor in descriptors::shortcut_descriptors() {
        if let Some(binding) = settings
            .input
            .shortcuts
            .keyboard
            .iter()
            .find(|binding| binding.action == descriptor.action)
            && let Some(key) = binding.key
        {
            by_key
                .entry(key)
                .or_default()
                .push(descriptor.label.to_string());
        }
    }

    by_key.retain(|_, labels| labels.len() > 1);
    by_key
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::declare_system_id;
    use nerust_gui_settings::{
        input::{KeyboardBinding, PersistedControlId, SystemInputSettings},
        shared::DesktopSharedSettings,
    };
    use nerust_input_traits::{
        AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
        DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor,
        PortId,
    };
    use nerust_keyboard::Key;

    use super::conflicting_keys;

    declare_system_id!(pub TestSysId, "test");

    const TEST_DEV: DeviceKindId = DeviceKindId::new("test.dev");
    const TEST_ATT: AttachmentId = AttachmentId::new("test.att");

    fn single_port_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor {
            ports: vec![PortDescriptor {
                id: PortId::new("p1"),
                label: "P1",
                attachments: vec![AttachmentSlotDescriptor {
                    id: TEST_ATT,
                    label: "P1",
                    device: TEST_DEV,
                    supported_devices: vec![TEST_DEV],
                }],
            }],
            devices: vec![DeviceDescriptor {
                kind: TEST_DEV,
                label: "Test Device",
                controls: vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: DigitalControlId::new("test.ctrl.a"),
                        label: "A",
                        description: "A button",
                    }),
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: DigitalControlId::new("test.ctrl.b"),
                        label: "B",
                        description: "B button",
                    }),
                ],
            }],
        }
    }

    #[test]
    fn no_conflicts_with_empty_settings() {
        let settings = DesktopSharedSettings::default();
        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn same_key_different_control_detected() {
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyZ,
        ));
        settings.input.systems.insert(Box::new(TestSysId), sys);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(conflicts.contains_key(&Key::KeyZ));
    }

    #[test]
    fn different_keys_not_conflicting() {
        let mut settings = DesktopSharedSettings::default();
        let mut sys = SystemInputSettings::default();
        let profile = sys.implicit_keyboard_profile_mut();
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.a".to_string()),
            Key::KeyZ,
        ));
        profile.bindings.push(KeyboardBinding::new(
            TEST_ATT.to_string(),
            PersistedControlId::digital("test.ctrl.b".to_string()),
            Key::KeyX,
        ));
        settings.input.systems.insert(Box::new(TestSysId), sys);

        let conflicts = conflicting_keys(&settings, &single_port_topology(), &TestSysId);
        assert!(!conflicts.contains_key(&Key::KeyZ));
        assert!(!conflicts.contains_key(&Key::KeyX));
    }
}
