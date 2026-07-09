use std::collections::HashSet;

use nerust_input_traits::{
    ControllerProfile, CreateSplitError, InputAssignments, InputPorts, InputResources, InputSplit,
    InputSystemFactory, SlotInfo,
};
use nerust_nes_controller::input_buffer::NesInputBuffer;
use nerust_nes_device::controller_profiles::{FAMICOM_SET_PROFILE, STANDARD_PAD_PROFILE};

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
        crate::nes_device_controller_profiles()
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

        let mut field_map = std::collections::HashMap::new();
        let mut assigned_ports = HashSet::new();

        for (slot_id, ctrl_opt) in &assignments.slots {
            let ctrl_id = match ctrl_opt {
                Some(id) => id.as_str(),
                None => continue,
            };
            let slot_key: &'static str = match slot_id.as_str() {
                "player1" => "player1",
                "player2" => "player2",
                _ => continue,
            };
            if !assigned_ports.insert(slot_key) {
                return Err(CreateSplitError::SlotConflict {
                    a: slot_key.to_string(),
                    b: slot_key.to_string(),
                });
            }
            let (profile, port_groups_list): (
                &dyn ControllerProfile,
                &[&[nerust_input_traits::ControlInfo]],
            ) = match ctrl_id {
                "nes.standard_pad" => (&STANDARD_PAD_PROFILE, STANDARD_PAD_PROFILE.port_groups()),
                "nes.famicom" => (&FAMICOM_SET_PROFILE, FAMICOM_SET_PROFILE.port_groups()),
                _ => {
                    return Err(CreateSplitError::ControllerNotFound {
                        controller: ctrl_id.to_string(),
                    });
                }
            };
            for ps in profile.port_sets() {
                if let Some(pos) = ps.ports.iter().position(|&p| p == slot_key) {
                    // Single-port controller: assign to this slot only
                    let controls = port_groups_list[pos];
                    let base = pos * 8;
                    for ci in controls {
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
                        field_map.insert((slot_key, ci.id), base + bit);
                    }
                    // Handle multi-port: also occupy other ports in the set
                    if ps.ports.len() > 1 {
                        for (gi, &port) in ps.ports.iter().enumerate() {
                            if port == slot_key {
                                continue;
                            }
                            if !assigned_ports.insert(port) {
                                return Err(CreateSplitError::SlotConflict {
                                    a: port.to_string(),
                                    b: port.to_string(),
                                });
                            }
                            let controls = port_groups_list[gi];
                            let base = gi * 8;
                            for ci in controls {
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
                                field_map.insert((port, ci.id), base + bit);
                            }
                        }
                        // Microphone for FamicomSet
                        if ctrl_id == "nes.famicom" {
                            field_map.insert(("player2", "microphone"), 16);
                        }
                    }
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
