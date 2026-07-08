use nerust_input_traits::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, InputAssignments,
    InputPorts, InputResources, InputSplit, InputSystemFactory, PortSet, SlotInfo,
    CreateSplitError,
};
use nerust_nes_controller::input_buffer::NesInputBuffer;

// ── ControllerProfile implementations ──

#[derive(Debug)]
struct StandardPad;
#[derive(Debug)]
struct FamicomSet;
#[derive(Debug)]
struct Zapper;

impl ControllerProfile for StandardPad {
    fn id(&self) -> &'static str { "nes.standard_pad" }
    fn label(&self) -> &'static str { "NES Standard Controller" }
    fn port_sets(&self) -> &[PortSet] {
        &[
            PortSet { ports: &["player1"] },
            PortSet { ports: &["player2"] },
        ]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use AbstractKey::*;
        use ControlKind::*;
        static CONTROLS: &[ControlInfo] = &[
            ControlInfo { id: "a", label: "A", kind: Digital, abstract_key: Some(Button1) },
            ControlInfo { id: "b", label: "B", kind: Digital, abstract_key: Some(Button2) },
            ControlInfo { id: "select", label: "Select", kind: Digital, abstract_key: Some(Select) },
            ControlInfo { id: "start", label: "Start", kind: Digital, abstract_key: Some(Start) },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(DpadRight) },
        ];
        static GROUPS: &[&[ControlInfo]] = &[CONTROLS];
        GROUPS
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        static DIRS: &[&[&'static str; 4]] = &[&["up", "down", "left", "right"]];
        DIRS
    }
}

impl ControllerProfile for FamicomSet {
    fn id(&self) -> &'static str { "nes.famicom_set" }
    fn label(&self) -> &'static str { "Famicom Controller Set" }
    fn port_sets(&self) -> &[PortSet] {
        &[PortSet { ports: &["player1", "player2"] }]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use AbstractKey::*;
        use ControlKind::*;
        static P1: &[ControlInfo] = &[
            ControlInfo { id: "a", label: "A", kind: Digital, abstract_key: Some(Button1) },
            ControlInfo { id: "b", label: "B", kind: Digital, abstract_key: Some(Button2) },
            ControlInfo { id: "select", label: "Select", kind: Digital, abstract_key: Some(Select) },
            ControlInfo { id: "start", label: "Start", kind: Digital, abstract_key: Some(Start) },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(DpadRight) },
        ];
        static P2: &[ControlInfo] = &[
            ControlInfo { id: "a", label: "A", kind: Digital, abstract_key: Some(Button1) },
            ControlInfo { id: "b", label: "B", kind: Digital, abstract_key: Some(Button2) },
            ControlInfo { id: "mic", label: "Microphone", kind: Digital, abstract_key: None },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(DpadRight) },
        ];
        static GROUPS: &[&[ControlInfo]] = &[P1, P2];
        GROUPS
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        static DIRS: &[&[&'static str; 4]] = &[&["up", "down", "left", "right"]];
        DIRS
    }
}

impl ControllerProfile for Zapper {
    fn id(&self) -> &'static str { "nes.zapper" }
    fn label(&self) -> &'static str { "NES Zapper" }
    fn port_sets(&self) -> &[PortSet] {
        &[
            PortSet { ports: &["player1"] },
            PortSet { ports: &["player2"] },
            PortSet { ports: &["expansion"] },
        ]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use ControlKind::*;
        static CONTROLS: &[ControlInfo] = &[
            ControlInfo { id: "trigger", label: "Trigger", kind: Digital, abstract_key: None },
        ];
        static GROUPS: &[&[ControlInfo]] = &[CONTROLS];
        GROUPS
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] { &[] }
}

// ── Static instances ──

pub static STANDARD_PAD: StandardPad = StandardPad;
pub static FAMICOM_SET: FamicomSet = FamicomSet;
pub static ZAPPER: Zapper = Zapper;

/// All NES controller profiles.
pub static NES_CONTROLLERS: &[&'static dyn ControllerProfile] = &[&STANDARD_PAD, &FAMICOM_SET, &ZAPPER];

// ── NesFactory InputSystemFactory impl ──

use std::sync::atomic::AtomicBool;

impl InputPorts for crate::NesFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[
            SlotInfo { id: "player1", label: "Player 1" },
            SlotInfo { id: "player2", label: "Player 2" },
            SlotInfo { id: "expansion", label: "Expansion" },
        ];
        SLOTS
    }
    fn controllers(&self) -> &[&'static dyn ControllerProfile] {
        NES_CONTROLLERS
    }
}

impl InputSystemFactory for crate::NesFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments {
            slots: vec![
                ("player1", Some("nes.standard_pad")),
                ("player2", Some("nes.standard_pad")),
                ("expansion", None),
            ],
        }
    }

    fn create_split(
        &self,
        assignments: &InputAssignments,
    ) -> Result<InputResources, CreateSplitError> {
        use std::sync::{Arc, Mutex};
        use nerust_input_traits::InputStateBuffer;

        let mut field_map = std::collections::HashMap::new();
        let mut field_index = 0usize;

        for (slot_id, ctrl_opt) in &assignments.slots {
            let ctrl_id = match ctrl_opt {
                Some(id) => *id,
                None => continue,
            };
            let slot_str: &str = slot_id;
            match ctrl_id {
                "nes.standard_pad" | "nes.famicom_p2" => {
                    let controls = if ctrl_id == "nes.famicom_p2" {
                        FamicomSet.port_groups()[1]
                    } else {
                        StandardPad.port_groups()[0]
                    };
                    for ci in controls {
                        field_map.insert((slot_str, ci.id), field_index);
                        field_index += 1;
                    }
                }
                "nes.famicom_set" => {
                    for (gi, controls) in FamicomSet.port_groups().iter().enumerate() {
                        let slot = if gi == 0 { "player1" } else { "player2" };
                        for ci in controls.iter() {
                            field_map.insert((slot, ci.id), field_index);
                            field_index += 1;
                        }
                    }
                }
                "nes.zapper" => {
                    for ci in Zapper.port_groups()[0].iter() {
                        field_map.insert((slot_str, ci.id), field_index);
                        field_index += 1;
                    }
                }
                _ => return Err(CreateSplitError::ControllerNotFound {
                    controller: ctrl_id.to_string(),
                }),
            }
        }

        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        let flag = Arc::new(AtomicBool::new(false));

        let split = InputSplit {
            shared: Arc::clone(&shared),
            flag: Arc::clone(&flag),
            new_buffer: Box::new(|| Box::<NesInputBuffer>::default()),
        };

        Ok(InputResources { split, field_map })
    }
}
