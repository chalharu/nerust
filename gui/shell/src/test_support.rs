use nerust_input_traits::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
    DigitalControlDescriptor, DigitalControlId, DigitalInputEvent, DigitalInputState,
    InputTopologyDescriptor, PortDescriptor, PortId,
};

// Shared test constants for input topology construction.
pub const TEST_ATT_P1: AttachmentId = AttachmentId::new("nes.attachment.player1");
pub const TEST_ATT_P2: AttachmentId = AttachmentId::new("nes.attachment.player2");
pub const TEST_DEV_P1: DeviceKindId = DeviceKindId::new("nes.device.player1_pad");
pub const TEST_DEV_P2: DeviceKindId = DeviceKindId::new("nes.device.player2_famicom_pad");
pub const TEST_CTRL_A: DigitalControlId = DigitalControlId::new("nes.control.a");
pub const TEST_CTRL_B: DigitalControlId = DigitalControlId::new("nes.control.b");
pub const TEST_CTRL_MIC: DigitalControlId = DigitalControlId::new("nes.control.microphone");

/// Single-port single-device topology (player 1 only).
pub fn single_port_topology() -> InputTopologyDescriptor {
    InputTopologyDescriptor {
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

/// Dual-port dual-device topology (player 1 + player 2 with microphone).
pub fn dual_port_topology() -> InputTopologyDescriptor {
    InputTopologyDescriptor {
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

/// Test resolver for keyboard bindings.
/// Maps known attachment/control strings to static IDs.
pub fn test_resolve(attachment: &str, control: &str, pressed: bool) -> Option<DigitalInputEvent> {
    let state = if pressed {
        DigitalInputState::Pressed
    } else {
        DigitalInputState::Released
    };
    let att = match attachment {
        "nes.attachment.player1" => TEST_ATT_P1,
        "nes.attachment.player2" => TEST_ATT_P2,
        _ => return None,
    };
    let ctrl = match control {
        "nes.control.a" => TEST_CTRL_A,
        "nes.control.microphone" => TEST_CTRL_MIC,
        _ => return None,
    };
    Some(DigitalInputEvent::new(att, ctrl, state))
}
