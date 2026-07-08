use nerust_input_traits::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSplit, InputSystemFactory, PortSet, SlotInfo,
};
use nerust_nes_controller::input_buffer::NesInputBuffer;

#[derive(Debug)]
struct FamicomSet;

impl ControllerProfile for FamicomSet {
    fn id(&self) -> &'static str {
        "nes.famicom"
    }
    fn label(&self) -> &'static str {
        "Famicom Controller Set"
    }
    fn port_sets(&self) -> &[PortSet] {
        &[PortSet {
            ports: &["player1", "player2"],
        }]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use AbstractKey::*;
        use ControlKind::*;
        static P1: &[ControlInfo] = &[
            ControlInfo {
                id: "a",
                label: "A",
                kind: Digital,
                abstract_key: Some(Button1),
            },
            ControlInfo {
                id: "b",
                label: "B",
                kind: Digital,
                abstract_key: Some(Button2),
            },
            ControlInfo {
                id: "select",
                label: "Select",
                kind: Digital,
                abstract_key: Some(Select),
            },
            ControlInfo {
                id: "start",
                label: "Start",
                kind: Digital,
                abstract_key: Some(Start),
            },
            ControlInfo {
                id: "up",
                label: "Up",
                kind: Digital,
                abstract_key: Some(DpadUp),
            },
            ControlInfo {
                id: "down",
                label: "Down",
                kind: Digital,
                abstract_key: Some(DpadDown),
            },
            ControlInfo {
                id: "left",
                label: "Left",
                kind: Digital,
                abstract_key: Some(DpadLeft),
            },
            ControlInfo {
                id: "right",
                label: "Right",
                kind: Digital,
                abstract_key: Some(DpadRight),
            },
        ];
        static P2: &[ControlInfo] = &[
            ControlInfo {
                id: "a",
                label: "A",
                kind: Digital,
                abstract_key: Some(Button1),
            },
            ControlInfo {
                id: "b",
                label: "B",
                kind: Digital,
                abstract_key: Some(Button2),
            },
            ControlInfo {
                id: "microphone",
                label: "Microphone",
                kind: Digital,
                abstract_key: None,
            },
            ControlInfo {
                id: "up",
                label: "Up",
                kind: Digital,
                abstract_key: Some(DpadUp),
            },
            ControlInfo {
                id: "down",
                label: "Down",
                kind: Digital,
                abstract_key: Some(DpadDown),
            },
            ControlInfo {
                id: "left",
                label: "Left",
                kind: Digital,
                abstract_key: Some(DpadLeft),
            },
            ControlInfo {
                id: "right",
                label: "Right",
                kind: Digital,
                abstract_key: Some(DpadRight),
            },
        ];
        static GROUPS: &[&[ControlInfo]] = &[P1, P2];
        GROUPS
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}

#[derive(Debug)]
struct StandardPad;

impl ControllerProfile for StandardPad {
    fn id(&self) -> &'static str {
        "nes.standard_pad"
    }
    fn label(&self) -> &'static str {
        "NES Standard Controller"
    }
    fn port_sets(&self) -> &[PortSet] {
        &[
            PortSet {
                ports: &["player1"],
            },
            PortSet {
                ports: &["player2"],
            },
        ]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use AbstractKey::*;
        use ControlKind::*;
        static C: &[ControlInfo] = &[
            ControlInfo {
                id: "a",
                label: "A",
                kind: Digital,
                abstract_key: Some(Button1),
            },
            ControlInfo {
                id: "b",
                label: "B",
                kind: Digital,
                abstract_key: Some(Button2),
            },
            ControlInfo {
                id: "select",
                label: "Select",
                kind: Digital,
                abstract_key: Some(Select),
            },
            ControlInfo {
                id: "start",
                label: "Start",
                kind: Digital,
                abstract_key: Some(Start),
            },
            ControlInfo {
                id: "up",
                label: "Up",
                kind: Digital,
                abstract_key: Some(DpadUp),
            },
            ControlInfo {
                id: "down",
                label: "Down",
                kind: Digital,
                abstract_key: Some(DpadDown),
            },
            ControlInfo {
                id: "left",
                label: "Left",
                kind: Digital,
                abstract_key: Some(DpadLeft),
            },
            ControlInfo {
                id: "right",
                label: "Right",
                kind: Digital,
                abstract_key: Some(DpadRight),
            },
        ];
        static G: &[&[ControlInfo]] = &[C];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}

static STANDARD_PAD: StandardPad = StandardPad;
static FAMICOM_SET: FamicomSet = FamicomSet;
static NES_CONTROLLERS: &[&'static dyn ControllerProfile] = &[&FAMICOM_SET, &STANDARD_PAD];

impl InputPorts for crate::NesFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[
            SlotInfo {
                id: "player1",
                label: "Player 1",
            },
            SlotInfo {
                id: "player2",
                label: "Player 2",
            },
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
                ("player1".to_string(), Some("nes.standard_pad".to_string())),
                ("player2".to_string(), Some("nes.standard_pad".to_string())),
            ],
        }
    }

    fn create_split(
        &self,
        assignments: &InputAssignments,
    ) -> Result<InputResources, CreateSplitError> {
        use nerust_input_traits::InputStateBuffer;
        use std::sync::{Arc, Mutex};

        let mut field_map = std::collections::HashMap::new();
        let mut assigned_ports = std::collections::HashSet::new();

        for (slot_id, ctrl_opt) in &assignments.slots {
            let ctrl_id = match ctrl_opt {
                Some(id) => id.as_str(),
                None => continue,
            };
            let slot: &str = slot_id;
            if !assigned_ports.insert(slot) {
                return Err(CreateSplitError::SlotConflict {
                    a: slot.to_string(),
                    b: slot.to_string(),
                });
            }
            match ctrl_id {
                "nes.standard_pad" => {
                    // Map slot_id to &'static str for field_map keys
                    let slot_key: &'static str = match slot {
                        "player1" => "player1",
                        "player2" => "player2",
                        _ => continue,
                    };
                    let base = if slot_key == "player2" { 8 } else { 0 };
                    for ci in STANDARD_PAD.port_groups()[0] {
                        let bit = match ci.id {
                            "a" => 0,
                            "b" => 1,
                            "select" => 2,
                            "start" => 3,
                            "up" => 4,
                            "down" => 5,
                            "left" => 6,
                            "right" => 7,
                            _ => continue,
                        };
                        field_map.insert((slot_key, ci.id), base + bit);
                    }
                }
                "nes.famicom" => {
                    // Occupies {P1,P2}. Generate field_map for both.
                    for (gi, controls) in FAMICOM_SET.port_groups().iter().enumerate() {
                        let s = if gi == 0 { "player1" } else { "player2" };
                        if !assigned_ports.insert(s) {
                            return Err(CreateSplitError::SlotConflict {
                                a: s.to_string(),
                                b: slot.to_string(),
                            });
                        }
                        let base = gi * 8;
                        for ci in controls.iter() {
                            let bit = match ci.id {
                                "a" => 0,
                                "b" => 1,
                                "select" => 2,
                                "start" => 3,
                                "up" => 4,
                                "down" => 5,
                                "left" => 6,
                                "right" => 7,
                                "microphone" => continue,
                                _ => continue,
                            };
                            field_map.insert((s, ci.id), base + bit);
                        }
                    }
                    // Microphone uses dedicated byte at field 16.
                    field_map.insert(("player2", "microphone"), 16);
                }
                _ => {
                    return Err(CreateSplitError::ControllerNotFound {
                        controller: ctrl_id.to_string(),
                    });
                }
            }
        }

        if field_map.is_empty() {
            return Err(CreateSplitError::ControllerNotFound {
                controller: "none".to_string(),
            });
        }

        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        let flag = std::sync::atomic::AtomicBool::new(false);

        let split = InputSplit {
            shared: Arc::clone(&shared),
            flag: Arc::new(flag),
            new_buffer: Box::new(|| Box::<NesInputBuffer>::default()),
        };

        Ok(InputResources { split, field_map })
    }
}
