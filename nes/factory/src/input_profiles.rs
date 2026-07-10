use std::collections::HashSet;

use nerust_input_traits::{
    ControllerProfile, CreateSplitError, InputAssignments, InputPorts, InputResources, InputSplit,
    InputSystemFactory, SlotInfo,
};
use nerust_nes_core::input_types::NesInputBuffer;

fn control_bit(id: &str) -> Option<usize> {
    match id {
        "a" => Some(0),
        "b" => Some(1),
        "select" => Some(2),
        "start" => Some(3),
        "up" => Some(4),
        "down" => Some(5),
        "left" => Some(6),
        "right" => Some(7),
        "microphone" => None,
        _ => None,
    }
}

fn collect_control_fields(
    field_map: &mut std::collections::HashMap<(&'static str, &'static str), usize>,
    controls: &[nerust_input_traits::ControlInfo],
    base: usize,
    port: &'static str,
) {
    for ci in controls {
        if let Some(bit) = control_bit(ci.id) {
            field_map.insert((port, ci.id), base + bit);
        }
    }
}

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
    fn controllers(&self) -> Vec<Box<dyn ControllerProfile>> {
        nerust_nes_device::nes_device_controller_profiles()
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
            let controllers = self.controllers();
            let profile = controllers
                .iter()
                .find(|p| p.id() == ctrl_id)
                .ok_or_else(|| CreateSplitError::ControllerNotFound {
                    controller: ctrl_id.to_string(),
                })?;
            let port_groups_list = profile.port_groups();
            for ps in profile.port_sets() {
                if let Some(pos) = ps.ports.iter().position(|&p| p == slot_key) {
                    let base = pos * 8;
                    collect_control_fields(&mut field_map, port_groups_list[pos], base, slot_key);
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
                            let base = gi * 8;
                            collect_control_fields(
                                &mut field_map,
                                port_groups_list[gi],
                                base,
                                port,
                            );
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
