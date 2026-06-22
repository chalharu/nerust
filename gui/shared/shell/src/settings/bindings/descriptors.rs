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

    fn test_topology() -> InputTopologyDescriptor {
        InputTopologyDescriptor {
            system: SystemId::Nes,
            ports: vec![
                PortDescriptor {
                    id: PortId::new("test.port1"),
                    label: "Port 1",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: AttachmentId::new("nes.attachment.player1"),
                        label: "Player 1",
                        device: DeviceKindId::new("nes.device.player1_pad"),
                        supported_devices: vec![],
                    }],
                },
                PortDescriptor {
                    id: PortId::new("test.port2"),
                    label: "Port 2",
                    attachments: vec![AttachmentSlotDescriptor {
                        id: AttachmentId::new("nes.attachment.player2"),
                        label: "Player 2",
                        device: DeviceKindId::new("nes.device.player2_famicom_pad"),
                        supported_devices: vec![],
                    }],
                },
            ],
            devices: vec![
                DeviceDescriptor {
                    kind: DeviceKindId::new("nes.device.player1_pad"),
                    label: "NES Pad",
                    controls: vec![
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: DigitalControlId::new("nes.control.a"),
                            label: "A",
                            description: "",
                        }),
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: DigitalControlId::new("nes.control.b"),
                            label: "B",
                            description: "",
                        }),
                    ],
                },
                DeviceDescriptor {
                    kind: DeviceKindId::new("nes.device.player2_famicom_pad"),
                    label: "Famicom Pad",
                    controls: vec![
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: DigitalControlId::new("nes.control.a"),
                            label: "A",
                            description: "",
                        }),
                        ControlDescriptor::Digital(DigitalControlDescriptor {
                            id: DigitalControlId::new("nes.control.microphone"),
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
        assert_eq!(
            sections[0].attachment,
            AttachmentId::new("nes.attachment.player1")
        );
        assert_eq!(
            sections[1].attachment,
            AttachmentId::new("nes.attachment.player2")
        );
        assert!(
            sections[1]
                .bindings
                .iter()
                .any(|binding| binding.control == DigitalControlId::new("nes.control.microphone"))
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
