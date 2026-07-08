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

static FAMICOM_SET: FamicomSet = FamicomSet;
static NES_CONTROLLERS: &[&'static dyn ControllerProfile] = &[&FAMICOM_SET];

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
                ("player1".to_string(), Some("nes.famicom".to_string())),
                ("player2".to_string(), None),
            ],
        }
    }

    fn create_split(
        &self,
        assignments: &InputAssignments,
    ) -> Result<InputResources, CreateSplitError> {
        use nerust_input_traits::InputStateBuffer;
        use std::sync::{Arc, Mutex};

        // Validate: at least one slot must be assigned to FamicomSet
        let has_famicom = assignments
            .slots
            .iter()
            .any(|(_, c)| c.as_deref() == Some("nes.famicom"));
        if !has_famicom {
            return Err(CreateSplitError::ControllerNotFound {
                controller: "nes.famicom".to_string(),
            });
        }

        // FamicomSet port_set = {P1, P2}. Populate field_map using NES protocol bit positions.
        let mut field_map = std::collections::HashMap::new();
        for (gi, controls) in FAMICOM_SET.port_groups().iter().enumerate() {
            let slot = if gi == 0 { "player1" } else { "player2" };
            let base = gi * 8;
            for ci in controls.iter() {
                // Map control to its NES shift-register bit position.
                let bit = match ci.id {
                    "a" => 0, "b" => 1,
                    "select" => 2, "start" => 3,
                    "up" => 4, "down" => 5, "left" => 6, "right" => 7,
                    "microphone" => continue, // handled below as dedicated field
                    _ => continue,
                };
                field_map.insert((slot, ci.id), base + bit);
            }
        }
        // Microphone uses dedicated byte at field 16 (NesInputBuffer[2]).
        field_map.insert(("player2", "microphone"), 16);

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
