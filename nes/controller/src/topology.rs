use nerust_input_traits::{AttachmentId, DigitalControlId, PortId};

pub const NES_PORT_ONE: PortId = PortId::new("nes.port1");
pub const NES_PORT_TWO: PortId = PortId::new("nes.port2");
pub const NES_ATTACHMENT_PLAYER_ONE: AttachmentId = AttachmentId::new("nes.attachment.player1");
pub const NES_ATTACHMENT_PLAYER_TWO: AttachmentId = AttachmentId::new("nes.attachment.player2");
pub const NES_DEVICE_PLAYER_ONE_PAD: PortId = PortId::new("nes.device.player1_pad");
pub const NES_DEVICE_PLAYER_TWO_FAMICOM_PAD: PortId = PortId::new("nes.device.player2_famicom_pad");
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
