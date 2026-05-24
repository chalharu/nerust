use nerust_core::OpenBusReadResult;
use nerust_core::controller::Controller;
use nerust_input_schema::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
    DigitalControlDescriptor, DigitalControlId, DigitalInputEvent, DigitalInputState,
    InputTopologyDescriptor, PortDescriptor, PortId, SystemId,
};

const STANDARD_CONTROLLER_MAX_INDEX: usize = 8;
const CONTROLLER_STATE_SCHEMA_VERSION: u32 = 1;
const INPUT_STATE_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NesInputFrame {
    pub player_one: Buttons,
    pub player_two: Buttons,
    pub microphone: bool,
}

impl Default for NesInputFrame {
    fn default() -> Self {
        Self {
            player_one: Buttons::empty(),
            player_two: Buttons::empty(),
            microphone: false,
        }
    }
}

bitflags::bitflags! {
    #[derive(
        serde_derive::Serialize,
        serde_derive::Deserialize,
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
    )]
    pub struct Buttons: u8 {
        const A =      0b0000_0001;
        const B =      0b0000_0010;
        const SELECT = 0b0000_0100;
        const START =  0b0000_1000;
        const UP =     0b0001_0000;
        const DOWN =   0b0010_0000;
        const LEFT =   0b0100_0000;
        const RIGHT =  0b1000_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardControllerSnapshot {
    pub buttons: [Buttons; 2],
    pub microphone: bool,
    pub index1: usize,
    pub index2: usize,
    pub strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
pub struct StandardController {
    buttons: [Buttons; 2],
    microphone: bool,
    index1: usize,
    index2: usize,
    strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerStatePayload {
    schema_version: u32,
    buttons: [Buttons; 2],
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct InputStatePayload {
    schema_version: u32,
    player_one: Buttons,
    player_two: Buttons,
    microphone: bool,
}

#[derive(Debug, Default)]
pub struct NesInputState {
    held: NesInputFrame,
    dirty_player_one: bool,
    dirty_player_two: bool,
    dirty_microphone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NesDigitalTarget {
    PadOne(Buttons),
    PadTwo(Buttons),
    Microphone,
}

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

pub fn input_frame_from_snapshot(snapshot: StandardControllerSnapshot) -> NesInputFrame {
    NesInputFrame {
        player_one: snapshot.buttons[0],
        player_two: snapshot.buttons[1],
        microphone: snapshot.microphone,
    }
}

pub fn snapshot_with_input_frame(
    snapshot: StandardControllerSnapshot,
    frame: NesInputFrame,
) -> StandardControllerSnapshot {
    StandardControllerSnapshot {
        buttons: [frame.player_one, frame.player_two],
        microphone: frame.microphone,
        ..snapshot
    }
}

pub fn encode_controller_state(snapshot: StandardControllerSnapshot) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(&ControllerStatePayload {
        schema_version: CONTROLLER_STATE_SCHEMA_VERSION,
        buttons: snapshot.buttons,
        microphone: snapshot.microphone,
        index1: snapshot.index1 as u64,
        index2: snapshot.index2 as u64,
        strobe: snapshot.strobe,
    })
    .map_err(|error| error.to_string())
}

pub fn decode_controller_state(bytes: &[u8]) -> Result<StandardControllerSnapshot, String> {
    let payload = rmp_serde::from_slice::<ControllerStatePayload>(bytes)
        .map_err(|error| error.to_string())?;
    if payload.schema_version != CONTROLLER_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported controller state schema version: {}",
            payload.schema_version
        ));
    }
    let index1 = usize::try_from(payload.index1).map_err(|_| "controller port1 index overflow")?;
    let index2 = usize::try_from(payload.index2).map_err(|_| "controller port2 index overflow")?;
    if index1 > STANDARD_CONTROLLER_MAX_INDEX {
        return Err("controller port1 index out of range".into());
    }
    if index2 > STANDARD_CONTROLLER_MAX_INDEX {
        return Err("controller port2 index out of range".into());
    }
    Ok(StandardControllerSnapshot {
        buttons: payload.buttons,
        microphone: payload.microphone,
        index1,
        index2,
        strobe: payload.strobe,
    })
}

pub fn encode_input_state(frame: NesInputFrame) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(&InputStatePayload {
        schema_version: INPUT_STATE_SCHEMA_VERSION,
        player_one: frame.player_one,
        player_two: frame.player_two,
        microphone: frame.microphone,
    })
    .map_err(|error| error.to_string())
}

pub fn decode_input_state(bytes: &[u8]) -> Result<NesInputFrame, String> {
    let payload =
        rmp_serde::from_slice::<InputStatePayload>(bytes).map_err(|error| error.to_string())?;
    if payload.schema_version != INPUT_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported input state schema version: {}",
            payload.schema_version
        ));
    }
    Ok(NesInputFrame {
        player_one: payload.player_one,
        player_two: payload.player_two,
        microphone: payload.microphone,
    })
}

impl StandardController {
    pub fn new() -> Self {
        Self {
            buttons: [Buttons::empty(); 2],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        }
    }

    pub fn reset(&mut self) {
        self.buttons = [Buttons::empty(); 2];
        self.microphone = false;
        self.index1 = 0;
        self.index2 = 0;
        self.strobe = false;
    }

    pub fn export_snapshot(&self) -> StandardControllerSnapshot {
        StandardControllerSnapshot {
            buttons: self.buttons,
            microphone: self.microphone,
            index1: self.index1,
            index2: self.index2,
            strobe: self.strobe,
        }
    }

    pub fn import_snapshot(&mut self, snapshot: StandardControllerSnapshot) {
        self.buttons = snapshot.buttons;
        self.microphone = snapshot.microphone;
        self.index1 = snapshot.index1;
        self.index2 = snapshot.index2;
        self.strobe = snapshot.strobe;
    }
}

impl Default for StandardController {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for StandardController {
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            0 => OpenBusReadResult::new(
                if self.index1 < 8 {
                    let result = self.buttons[0].bits() >> self.index1;
                    if !self.strobe {
                        self.index1 += 1;
                    }
                    result & 1
                } else {
                    1
                } | (if self.microphone { 0x04 } else { 0 }),
                7,
            ),
            1 => OpenBusReadResult::new(
                if self.index2 < 8 {
                    let result = self.buttons[1].bits() >> self.index2;
                    if !self.strobe {
                        self.index2 += 1;
                    }
                    result & 1
                } else {
                    1
                },
                0x1F,
            ),
            _ => {
                log::error!("unhandled controller read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    fn write(&mut self, value: u8) {
        self.strobe = value & 1 == 1;
        if self.strobe {
            self.index1 = 0;
            self.index2 = 0;
        }
    }
}

impl NesInputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_input(&mut self, event: DigitalInputEvent) {
        let Some(target) = Self::digital_target(event.attachment, event.control) else {
            return;
        };
        match target {
            NesDigitalTarget::PadOne(flag) => {
                self.held.player_one =
                    Self::apply_button_state(self.held.player_one, flag, event.state);
                self.dirty_player_one = true;
            }
            NesDigitalTarget::PadTwo(flag) => {
                self.held.player_two =
                    Self::apply_button_state(self.held.player_two, flag, event.state);
                self.dirty_player_two = true;
            }
            NesDigitalTarget::Microphone => {
                self.held.microphone = matches!(event.state, DigitalInputState::Pressed);
                self.dirty_microphone = true;
            }
        }
    }

    pub fn flush_to_snapshot(
        &self,
        snapshot: StandardControllerSnapshot,
    ) -> StandardControllerSnapshot {
        let mut frame = input_frame_from_snapshot(snapshot);
        if self.dirty_player_one {
            frame.player_one = self.held.player_one;
        }
        if self.dirty_player_two {
            frame.player_two = self.held.player_two;
        }
        if self.dirty_microphone {
            frame.microphone = self.held.microphone;
        }
        snapshot_with_input_frame(snapshot, frame)
    }

    pub fn clear(&mut self, snapshot: StandardControllerSnapshot) -> StandardControllerSnapshot {
        if self.dirty_player_one {
            self.held.player_one = Buttons::empty();
        }
        if self.dirty_player_two {
            self.held.player_two = Buttons::empty();
        }
        if self.dirty_microphone {
            self.held.microphone = false;
        }
        self.flush_to_snapshot(snapshot)
    }

    pub fn clear_current_frame(&mut self) -> NesInputFrame {
        if self.dirty_player_one {
            self.held.player_one = Buttons::empty();
        }
        if self.dirty_player_two {
            self.held.player_two = Buttons::empty();
        }
        if self.dirty_microphone {
            self.held.microphone = false;
        }
        self.held
    }

    pub fn sync_from_snapshot(&mut self, snapshot: StandardControllerSnapshot) {
        self.held = input_frame_from_snapshot(snapshot);
        self.dirty_player_one = false;
        self.dirty_player_two = false;
        self.dirty_microphone = false;
    }

    pub fn current_frame(&self) -> NesInputFrame {
        self.held
    }

    fn digital_target(
        attachment: AttachmentId,
        control: DigitalControlId,
    ) -> Option<NesDigitalTarget> {
        let button = match control {
            NES_CONTROL_A => Some(Buttons::A),
            NES_CONTROL_B => Some(Buttons::B),
            NES_CONTROL_SELECT => Some(Buttons::SELECT),
            NES_CONTROL_START => Some(Buttons::START),
            NES_CONTROL_UP => Some(Buttons::UP),
            NES_CONTROL_DOWN => Some(Buttons::DOWN),
            NES_CONTROL_LEFT => Some(Buttons::LEFT),
            NES_CONTROL_RIGHT => Some(Buttons::RIGHT),
            _ => None,
        };

        match attachment {
            NES_ATTACHMENT_PLAYER_ONE => button.map(NesDigitalTarget::PadOne),
            NES_ATTACHMENT_PLAYER_TWO => {
                if control == FAMICOM_P2_CONTROL_MICROPHONE {
                    Some(NesDigitalTarget::Microphone)
                } else {
                    button.and_then(|flag| match control {
                        NES_CONTROL_SELECT | NES_CONTROL_START => None,
                        _ => Some(NesDigitalTarget::PadTwo(flag)),
                    })
                }
            }
            _ => None,
        }
    }

    fn apply_button_state(current: Buttons, flag: Buttons, state: DigitalInputState) -> Buttons {
        match state {
            DigitalInputState::Pressed => current | flag,
            DigitalInputState::Released => current & !flag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Buttons, FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE,
        NES_ATTACHMENT_PLAYER_TWO, NES_CONTROL_A, NES_CONTROL_RIGHT, NES_CONTROL_SELECT,
        NES_DEVICE_PLAYER_ONE_PAD, NES_DEVICE_PLAYER_TWO_FAMICOM_PAD, NesInputFrame, NesInputState,
        StandardController, StandardControllerSnapshot, decode_controller_state,
        decode_input_state, encode_controller_state, encode_input_state, input_topology_descriptor,
    };
    use nerust_core::controller::Controller;
    use nerust_input_schema::{ControlDescriptor, DigitalInputEvent};

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

    #[test]
    fn controller_state_round_trips() {
        let snapshot = StandardControllerSnapshot {
            buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let decoded =
            decode_controller_state(&encode_controller_state(snapshot).expect("encode succeeds"))
                .expect("decode succeeds");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn input_state_round_trips() {
        let frame = NesInputFrame {
            player_one: Buttons::A | Buttons::START,
            player_two: Buttons::LEFT,
            microphone: true,
        };

        let decoded = decode_input_state(&encode_input_state(frame).expect("encode succeeds"))
            .expect("decode succeeds");

        assert_eq!(decoded, frame);
    }

    #[test]
    fn nes_input_state_maps_player_one_buttons() {
        let mut input = NesInputState::new();

        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));
        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_RIGHT,
        ));
        input.handle_input(DigitalInputEvent::released(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        assert_eq!(
            input.current_frame(),
            NesInputFrame {
                player_one: Buttons::RIGHT,
                ..NesInputFrame::default()
            }
        );
    }

    #[test]
    fn nes_input_state_preserves_unowned_snapshot_fields() {
        let mut input = NesInputState::new();
        let base = StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };
        input.sync_from_snapshot(base);
        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_ONE,
            NES_CONTROL_A,
        ));

        assert_eq!(
            input.flush_to_snapshot(base),
            StandardControllerSnapshot {
                buttons: [Buttons::A, Buttons::LEFT],
                microphone: true,
                index1: 3,
                index2: 5,
                strobe: true,
            }
        );
    }

    #[test]
    fn nes_input_state_maps_microphone_without_accepting_select_on_player_two() {
        let mut input = NesInputState::new();

        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_TWO,
            FAMICOM_P2_CONTROL_MICROPHONE,
        ));
        input.handle_input(DigitalInputEvent::pressed(
            NES_ATTACHMENT_PLAYER_TWO,
            NES_CONTROL_SELECT,
        ));

        assert_eq!(
            input.current_frame(),
            NesInputFrame {
                microphone: true,
                ..NesInputFrame::default()
            }
        );
    }

    #[test]
    fn standard_controller_returns_one_after_eight_bits() {
        let mut controller = StandardController::new();
        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::A, Buttons::empty()],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        });

        controller.write(1);
        controller.write(0);

        for _ in 0..8 {
            let _ = controller.read(0);
        }

        assert_eq!(controller.read(0).data & 1, 1);
        assert_eq!(controller.read(0).data & 1, 1);
    }

    #[test]
    fn standard_controller_reports_microphone_on_port_zero_d2() {
        let mut controller = StandardController::new();

        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::empty()],
            microphone: true,
            index1: 0,
            index2: 0,
            strobe: false,
        });
        assert_eq!(controller.read(0).data & 0x04, 0x04);

        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::empty()],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        });
        assert_eq!(controller.read(0).data & 0x04, 0x00);
    }
}
