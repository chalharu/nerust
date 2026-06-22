pub mod descriptors;
pub mod events;
pub mod keys;

use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::shared::DesktopSharedSettings;
use nerust_input_schema::InputTopologyDescriptor;
use std::collections::BTreeMap;

pub fn conflicting_keys(
    settings: &DesktopSharedSettings,
    topology: &InputTopologyDescriptor,
) -> BTreeMap<KeyboardKey, Vec<String>> {
    let mut by_key = BTreeMap::<KeyboardKey, Vec<String>>::new();

    if let Some(profile) = settings
        .input
        .systems
        .get(&topology.system)
        .and_then(|system| system.implicit_keyboard_profile())
    {
        for descriptor in descriptors::keyboard_binding_descriptors(topology) {
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
    use super::conflicting_keys;
    use crate::settings::defaults::seed::default_shared_settings;
    use nerust_gui_settings::input::KeyboardKey;
    use nerust_input_schema::{
        AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
        DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor,
        PortId, SystemId,
    };

    const TEST_ATT_P1: AttachmentId = AttachmentId::new("nes.attachment.player1");
    const TEST_DEV_P1: DeviceKindId = DeviceKindId::new("nes.device.player1_pad");
    const TEST_CTRL_A: DigitalControlId = DigitalControlId::new("nes.control.a");
    const TEST_CTRL_B: DigitalControlId = DigitalControlId::new("nes.control.b");

    fn test_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor {
            system: SystemId::Nes,
            ports: vec![PortDescriptor {
                id: PortId::new("test.port1"),
                label: "Port 1",
                attachments: vec![AttachmentSlotDescriptor {
                    id: TEST_ATT_P1,
                    label: "Player 1",
                    device: TEST_DEV_P1,
                    supported_devices: vec![],
                }],
            }],
            devices: vec![DeviceDescriptor {
                kind: TEST_DEV_P1,
                label: "NES Pad",
                controls: vec![
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: TEST_CTRL_A,
                        label: "A",
                        description: "",
                    }),
                    ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: TEST_CTRL_B,
                        label: "B",
                        description: "",
                    }),
                ],
            }],
        }
    }

    #[test]
    fn detects_conflicts_across_controls_and_shortcuts() {
        let mut settings = default_shared_settings();
        settings
            .input
            .shortcuts
            .keyboard
            .iter_mut()
            .find(|binding| {
                matches!(
                    binding.action,
                    nerust_gui_settings::input::ShortcutAction::TogglePause
                )
            })
            .unwrap()
            .key = Some(KeyboardKey::KeyZ);

        let conflicts = conflicting_keys(&settings, &test_topology());
        assert!(conflicts.contains_key(&KeyboardKey::KeyZ));
    }
}
