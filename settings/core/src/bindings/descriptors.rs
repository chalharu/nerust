use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::input::ShortcutAction;
use nerust_input_traits::{
    AttachmentId, ControlDescriptor, DigitalControlId, InputTopologyDescriptor,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBindingDescriptor {
    pub system: Box<dyn SystemId>,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub control: DigitalControlId,
    pub control_label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBindingSectionDescriptor {
    pub system: Box<dyn SystemId>,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub bindings: Vec<KeyboardBindingDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    pub action: ShortcutAction,
    pub label: &'static str,
}

const SHORTCUT_DESCRIPTORS: [ShortcutDescriptor; 7] = [
    ShortcutDescriptor {
        action: ShortcutAction::TogglePause,
        label: "Toggle Pause",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SaveActiveSlot,
        label: "Save Active Slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectNextSlot,
        label: "Select Next Slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectPreviousSlot,
        label: "Select Previous Slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::LoadActiveSlot,
        label: "Load Active Slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::ToggleFullscreen,
        label: "Toggle Fullscreen",
    },
    ShortcutDescriptor {
        action: ShortcutAction::Reset,
        label: "Reset",
    },
];

pub fn keyboard_binding_descriptors(
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
) -> Vec<KeyboardBindingDescriptor> {
    let mut descriptors = Vec::new();
    for device in &topology.devices {
        for port in &topology.ports {
            for att in &port.attachments {
                if att.device != device.kind {
                    continue;
                }
                for control in &device.controls {
                    match control {
                        ControlDescriptor::Digital(dc) => {
                            descriptors.push(KeyboardBindingDescriptor {
                                system: system.clone_box(),
                                attachment: att.id,
                                attachment_label: port.label,
                                control: dc.id,
                                control_label: dc.label,
                            });
                        }
                        ControlDescriptor::Analog(_) => {} // keyboard bindings are digital only
                    }
                }
            }
        }
    }
    descriptors
}

pub fn keyboard_binding_sections(
    topology: &InputTopologyDescriptor,
    system: &dyn SystemId,
) -> Vec<KeyboardBindingSectionDescriptor> {
    let bindings = keyboard_binding_descriptors(topology, system);
    let mut sections: Vec<KeyboardBindingSectionDescriptor> = Vec::new();
    for binding in bindings {
        if let Some(section) = sections
            .iter_mut()
            .find(|s| s.attachment == binding.attachment)
        {
            section.bindings.push(binding);
        } else {
            let attachment = binding.attachment;
            let attachment_label = binding.attachment_label;
            sections.push(KeyboardBindingSectionDescriptor {
                system: system.clone_box(),
                attachment,
                attachment_label,
                bindings: vec![binding],
            });
        }
    }
    sections
}

pub fn shortcut_descriptors() -> &'static [ShortcutDescriptor] {
    &SHORTCUT_DESCRIPTORS
}

#[cfg(test)]
mod tests {
    use nerust_gui_settings::input::ShortcutAction;
    use nerust_input_traits::{
        AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
        DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor,
        PortId,
    };

    use super::{keyboard_binding_sections, shortcut_descriptors};

    nerust_core_traits::declare_system_id!(pub TestSysId, "test");

    const TEST_ATT_P1: AttachmentId = AttachmentId::new("test.slot.p1");
    const TEST_ATT_P2: AttachmentId = AttachmentId::new("test.slot.p2");
    const TEST_CTRL_A: DigitalControlId = DigitalControlId::new("test.control.a");
    const TEST_CTRL_B: DigitalControlId = DigitalControlId::new("test.control.b");
    const TEST_CTRL_MIC: DigitalControlId = DigitalControlId::new("test.control.mic");

    fn dual_port_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor {
            ports: vec![
                PortDescriptor {
                    id: PortId::new("p1"),
                    label: "P1",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: TEST_ATT_P1,
                        label: "P1",
                        device: DeviceKindId::new("nes.standard"),
                        supported_devices: vec![DeviceKindId::new("nes.standard")],
                    }],
                },
                PortDescriptor {
                    id: PortId::new("p2"),
                    label: "P2",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: TEST_ATT_P2,
                        label: "P2",
                        device: DeviceKindId::new("nes.standard"),
                        supported_devices: vec![DeviceKindId::new("nes.standard")],
                    }],
                },
            ],
            devices: vec![
                DeviceDescriptor {
                    kind: DeviceKindId::new("nes.standard"),
                    label: "NES Standard",
                    controls: vec![
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: TEST_CTRL_A,
                            label: "A",
                            description: "A button",
                        }),
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: TEST_CTRL_B,
                            label: "B",
                            description: "B button",
                        }),
                    ],
                },
                DeviceDescriptor {
                    kind: DeviceKindId::new("nes.famicom_p2"),
                    label: "Famicom P2 (mic)",
                    controls: vec![ControlDescriptor::Digital(DigitalControlDescriptor {
                        id: TEST_CTRL_MIC,
                        label: "Microphone",
                        description: "Microphone",
                    })],
                },
            ],
        }
    }

    #[test]
    fn topology_driven_sections_keep_player_boundaries() {
        let sections = keyboard_binding_sections(&dual_port_topology(), &TestSysId);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].attachment, TEST_ATT_P1);
        assert_eq!(sections[1].attachment, TEST_ATT_P2);
        // Both sections share the same device kind so they get the same controls
        assert!(
            sections[0]
                .bindings
                .iter()
                .any(|binding| binding.control == TEST_CTRL_A)
        );
        assert!(
            sections[1]
                .bindings
                .iter()
                .any(|binding| binding.control == TEST_CTRL_B)
        );
    }

    #[test]
    fn shortcuts_remain_stable() {
        assert!(
            shortcut_descriptors()
                .iter()
                .any(|descriptor| matches!(descriptor.action, ShortcutAction::ToggleFullscreen))
        );
    }
}
