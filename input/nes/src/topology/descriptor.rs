use super::ids::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP, NES_DEVICE_PLAYER_ONE_PAD,
    NES_DEVICE_PLAYER_TWO_FAMICOM_PAD, NES_PORT_ONE, NES_PORT_TWO,
};
use nerust_input_schema::{
    AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DigitalControlDescriptor,
    DigitalControlId, InputTopologyDescriptor, PortDescriptor, SystemId,
};

fn digital_control(
    id: DigitalControlId,
    label: &'static str,
    description: &'static str,
) -> ControlDescriptor {
    ControlDescriptor::Digital(DigitalControlDescriptor {
        id,
        label,
        description,
    })
}

pub fn input_topology_descriptor() -> InputTopologyDescriptor {
    InputTopologyDescriptor {
        system: SystemId::Nes,
        ports: vec![
            PortDescriptor {
                id: NES_PORT_ONE,
                label: "Controller Port 1",
                attachments: vec![AttachmentSlotDescriptor {
                    id: NES_ATTACHMENT_PLAYER_ONE,
                    label: "Player 1",
                    device: NES_DEVICE_PLAYER_ONE_PAD,
                    supported_devices: vec![NES_DEVICE_PLAYER_ONE_PAD],
                }],
            },
            PortDescriptor {
                id: NES_PORT_TWO,
                label: "Controller Port 2",
                attachments: vec![AttachmentSlotDescriptor {
                    id: NES_ATTACHMENT_PLAYER_TWO,
                    label: "Player 2",
                    device: NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
                    supported_devices: vec![NES_DEVICE_PLAYER_TWO_FAMICOM_PAD],
                }],
            },
        ],
        devices: vec![
            DeviceDescriptor {
                kind: NES_DEVICE_PLAYER_ONE_PAD,
                label: "NES Controller",
                controls: vec![
                    digital_control(NES_CONTROL_A, "A", "Face button A"),
                    digital_control(NES_CONTROL_B, "B", "Face button B"),
                    digital_control(NES_CONTROL_SELECT, "Select", "Select button"),
                    digital_control(NES_CONTROL_START, "Start", "Start button"),
                    digital_control(NES_CONTROL_UP, "Up", "D-pad Up"),
                    digital_control(NES_CONTROL_DOWN, "Down", "D-pad Down"),
                    digital_control(NES_CONTROL_LEFT, "Left", "D-pad Left"),
                    digital_control(NES_CONTROL_RIGHT, "Right", "D-pad Right"),
                ],
            },
            DeviceDescriptor {
                kind: NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
                label: "Famicom Player 2 Controller",
                controls: vec![
                    digital_control(NES_CONTROL_A, "A", "Face button A"),
                    digital_control(NES_CONTROL_B, "B", "Face button B"),
                    digital_control(NES_CONTROL_UP, "Up", "D-pad Up"),
                    digital_control(NES_CONTROL_DOWN, "Down", "D-pad Down"),
                    digital_control(NES_CONTROL_LEFT, "Left", "D-pad Left"),
                    digital_control(NES_CONTROL_RIGHT, "Right", "D-pad Right"),
                    digital_control(
                        FAMICOM_P2_CONTROL_MICROPHONE,
                        "Microphone",
                        "Famicom player 2 microphone",
                    ),
                ],
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::input_topology_descriptor;
    use crate::topology::ids::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD, NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
    };
    use nerust_input_schema::ControlDescriptor;

    #[test]
    fn nes_topology_reports_distinct_player_devices() {
        let descriptor = input_topology_descriptor();

        assert_eq!(descriptor.ports.len(), 2);
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_ONE)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_ONE_PAD
        );
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_TWO)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_TWO_FAMICOM_PAD
        );
    }

    #[test]
    fn nes_topology_keeps_microphone_only_on_player_two() {
        let descriptor = input_topology_descriptor();
        let player_two_controls = &descriptor
            .device(NES_DEVICE_PLAYER_TWO_FAMICOM_PAD)
            .unwrap()
            .controls;

        assert!(player_two_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == FAMICOM_P2_CONTROL_MICROPHONE
            )
        }));
        assert!(!player_two_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_SELECT
            )
        }));
    }
}
