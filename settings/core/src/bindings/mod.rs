pub mod descriptors;

use std::collections::BTreeMap;

use crate::model::shared::DesktopSharedSettings;
use nerust_core_traits::identity::SystemId;
use nerust_input_traits::InputTopologyDescriptor;
use nerust_keyboard::Key;

/// Find key binding conflicts within a single system.
/// Only the implicit keyboard profile is checked; same-key shortcut conflicts included.
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
    use crate::model::{
        input::{KeyboardBinding, PersistedControlId, SystemInputSettings},
        shared::DesktopSharedSettings,
    };
    use nerust_core_traits::declare_system_id;
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
        InputTopologyDescriptor::new(
            vec![PortDescriptor::new(
                PortId::new("p1"),
                "P1",
                vec![AttachmentSlotDescriptor::new(
                    TEST_ATT,
                    "P1",
                    TEST_DEV,
                    vec![TEST_DEV],
                )],
            )],
            vec![DeviceDescriptor::new(
                TEST_DEV,
                "Test Device",
                vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor::new(
                        DigitalControlId::new("test.ctrl.a"),
                        "A",
                        "A button",
                    )),
                    ControlDescriptor::Digital(DigitalControlDescriptor::new(
                        DigitalControlId::new("test.ctrl.b"),
                        "B",
                        "B button",
                    )),
                ],
            )],
        )
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
