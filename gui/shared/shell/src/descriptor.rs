use nerust_console::Console;
use nerust_gui_runtime::session::{ConsoleSessionFactory, GuiSession};
use nerust_gui_session::core::SessionCore;
use nerust_input_schema::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
    DigitalControlDescriptor, DigitalControlId, InputTopologyDescriptor, PortDescriptor, PortId,
    SystemId,
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;

pub const NES_PORT_ONE: PortId = PortId::new("nes.port1");
pub const NES_PORT_TWO: PortId = PortId::new("nes.port2");
pub const NES_ATTACHMENT_PLAYER_ONE: AttachmentId = AttachmentId::new("nes.attachment.player1");
pub const NES_ATTACHMENT_PLAYER_TWO: AttachmentId = AttachmentId::new("nes.attachment.player2");
pub const NES_DEVICE_PLAYER_ONE_PAD: DeviceKindId = DeviceKindId::new("nes.device.player1_pad");
pub const NES_DEVICE_PLAYER_TWO_FAMICOM_PAD: DeviceKindId =
    DeviceKindId::new("nes.device.player2_famicom_pad");
pub const NES_CONTROL_A: DigitalControlId = DigitalControlId::new("nes.control.a");
pub const NES_CONTROL_B: DigitalControlId = DigitalControlId::new("nes.control.b");
pub const NES_CONTROL_SELECT: DigitalControlId = DigitalControlId::new("nes.control.select");
pub const NES_CONTROL_START: DigitalControlId = DigitalControlId::new("nes.control.start");
pub const NES_CONTROL_UP: DigitalControlId = DigitalControlId::new("nes.control.up");
pub const NES_CONTROL_DOWN: DigitalControlId = DigitalControlId::new("nes.control.down");
pub const NES_CONTROL_LEFT: DigitalControlId = DigitalControlId::new("nes.control.left");
pub const NES_CONTROL_RIGHT: DigitalControlId = DigitalControlId::new("nes.control.right");
pub const FAMICOM_P2_CONTROL_MICROPHONE: DigitalControlId =
    DigitalControlId::new("nes.control.microphone");

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

#[derive(Debug, Clone, Copy, Default)]
pub struct NesConsoleDescriptor;

impl NesConsoleDescriptor {
    pub fn build_console(self) -> Console {
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        Console::new(speaker, ScreenBuffer::new_nes_gpu_default())
    }

    pub fn build_session(&self) -> GuiSession {
        let core = SessionCore::from_console(self.build_console());
        GuiSession::from_session_core(core)
    }

    pub fn input_topology_descriptor(&self) -> InputTopologyDescriptor {
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
}

impl ConsoleSessionFactory for NesConsoleDescriptor {
    fn build_session(&self) -> GuiSession {
        NesConsoleDescriptor::build_session(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD,
        NES_DEVICE_PLAYER_TWO_FAMICOM_PAD, NesConsoleDescriptor,
    };
    use nerust_input_schema::ControlDescriptor;

    #[test]
    fn nes_descriptor_reports_distinct_player_one_and_player_two_devices() {
        let descriptor = NesConsoleDescriptor.input_topology_descriptor();

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
    fn nes_descriptor_keeps_select_on_player_one_and_microphone_on_player_two() {
        let descriptor = NesConsoleDescriptor.input_topology_descriptor();
        let player_one_controls = &descriptor
            .device(NES_DEVICE_PLAYER_ONE_PAD)
            .unwrap()
            .controls;
        let player_two_controls = &descriptor
            .device(NES_DEVICE_PLAYER_TWO_FAMICOM_PAD)
            .unwrap()
            .controls;

        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_A
            )
        }));
        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_SELECT
            )
        }));
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
