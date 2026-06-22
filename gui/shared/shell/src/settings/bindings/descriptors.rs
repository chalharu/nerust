use nerust_gui_settings::input::ShortcutAction;
use nerust_input_schema::{
    AttachmentId, ControlDescriptor, DigitalControlId, InputTopologyDescriptor, SystemId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardBindingDescriptor {
    pub system: SystemId,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub control: DigitalControlId,
    pub control_label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBindingSectionDescriptor {
    pub system: SystemId,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub bindings: Vec<KeyboardBindingDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    pub action: ShortcutAction,
    pub label: &'static str,
}

const SHORTCUT_DESCRIPTORS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        action: ShortcutAction::TogglePause,
        label: "Toggle pause",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SaveActiveSlot,
        label: "Save active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectNextSlot,
        label: "Select next slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectPreviousSlot,
        label: "Select previous slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::LoadActiveSlot,
        label: "Load active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::ToggleFullscreen,
        label: "Toggle fullscreen",
    },
    ShortcutDescriptor {
        action: ShortcutAction::Reset,
        label: "Reset",
    },
];

pub fn keyboard_binding_descriptors(
    topology: &InputTopologyDescriptor,
) -> Vec<KeyboardBindingDescriptor> {
    keyboard_binding_sections(topology)
        .into_iter()
        .flat_map(|section| section.bindings)
        .collect()
}

pub fn keyboard_binding_sections(
    topology: &InputTopologyDescriptor,
) -> Vec<KeyboardBindingSectionDescriptor> {
    topology
        .ports
        .iter()
        .flat_map(|port| port.attachments.iter())
        .filter_map(|attachment| {
            let device = topology.device(attachment.device)?;
            let bindings = device
                .controls
                .iter()
                .filter_map(|control| match control {
                    ControlDescriptor::Digital(control) => Some(KeyboardBindingDescriptor {
                        system: topology.system,
                        attachment: attachment.id,
                        attachment_label: attachment.label,
                        control: control.id,
                        control_label: control.label,
                    }),
                    ControlDescriptor::Analog(_) => None,
                })
                .collect::<Vec<_>>();
            Some(KeyboardBindingSectionDescriptor {
                system: topology.system,
                attachment: attachment.id,
                attachment_label: attachment.label,
                bindings,
            })
        })
        .collect()
}

pub fn shortcut_descriptors() -> &'static [ShortcutDescriptor] {
    SHORTCUT_DESCRIPTORS
}

#[cfg(test)]
mod tests {
    use super::{keyboard_binding_sections, shortcut_descriptors};
    use nerust_input_schema::{
        AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
        DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor,
        PortId, SystemId,
    };

    const TEST_ATT_P1: AttachmentId = AttachmentId::new("nes.attachment.player1");
    const TEST_ATT_P2: AttachmentId = AttachmentId::new("nes.attachment.player2");
    const TEST_DEV_P1: DeviceKindId = DeviceKindId::new("nes.device.player1_pad");
    const TEST_DEV_P2: DeviceKindId = DeviceKindId::new("nes.device.player2_famicom_pad");
    const TEST_CTRL_A: DigitalControlId = DigitalControlId::new("nes.control.a");
    const TEST_CTRL_B: DigitalControlId = DigitalControlId::new("nes.control.b");
    const TEST_CTRL_MIC: DigitalControlId = DigitalControlId::new("nes.control.microphone");

    fn test_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor {
            system: SystemId::Nes,
            ports: vec![
                PortDescriptor {
                    id: PortId::new("test.port1"),
                    label: "Port 1",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: TEST_ATT_P1,
                        label: "Player 1",
                        device: TEST_DEV_P1,
                        supported_devices: vec![],
                    }],
                },
                PortDescriptor {
                    id: PortId::new("test.port2"),
                    label: "Port 2",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: TEST_ATT_P2,
                        label: "Player 2",
                        device: TEST_DEV_P2,
                        supported_devices: vec![],
                    }],
                },
            ],
            devices: vec![
                DeviceDescriptor {
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
                },
                DeviceDescriptor {
                    kind: TEST_DEV_P2,
                    label: "Famicom Pad",
                    controls: vec![
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: TEST_CTRL_A,
                            label: "A",
                            description: "",
                        }),
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: TEST_CTRL_MIC,
                            label: "Microphone",
                            description: "",
                        }),
                    ],
                },
            ],
        }
    }

    #[test]
    fn topology_driven_sections_keep_player_boundaries() {
        let sections = keyboard_binding_sections(&test_topology());

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].attachment, TEST_ATT_P1);
        assert_eq!(sections[1].attachment, TEST_ATT_P2);
        assert!(
            sections[1]
                .bindings
                .iter()
                .any(|binding| binding.control == TEST_CTRL_MIC)
        );
    }

    #[test]
    fn shortcuts_remain_stable() {
        assert!(shortcut_descriptors().iter().any(|descriptor| matches!(
            descriptor.action,
            nerust_gui_settings::input::ShortcutAction::ToggleFullscreen
        )));
    }
}
