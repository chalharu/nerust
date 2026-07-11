use std::rc::Rc;

use nerust_input_traits::{
    ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments, InputPorts,
    InputResources, InputSplit, InputSystemFactory, SlotInfo,
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
    fn controllers(&self) -> Vec<Rc<dyn ControllerProfile>> {
        nerust_nes_device::nes_device_controller_profiles()
    }
}

impl InputSystemFactory for crate::NesFactory {
    fn default_assignments(&self) -> InputAssignments {
        let profiles = self.controllers();
        let famicom = profiles.iter().find(|p| p.id() == "nes.famicom").cloned();
        InputAssignments {
            slots: vec![
                ("player1".to_string(), famicom),
                ("player2".to_string(), None),
            ],
        }
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        use nerust_input_traits::InputStateBuffer;
        use std::sync::{Arc, Mutex};

        let mut field_map = std::collections::HashMap::new();

        for (_, profile_opt) in controllers.profiles.iter().enumerate() {
            let profile = match profile_opt {
                Some(p) => p.as_ref(),
                None => continue,
            };
            let groups = profile.port_groups();
            for (gi, &port) in profile
                .port_sets()
                .iter()
                .flat_map(|ps| ps.ports.iter().enumerate())
            {
                let base = gi * 8;
                if let Some(controls) = groups.get(gi) {
                    collect_control_fields(&mut field_map, controls, base, port);
                }
            }
            // Microphone for FamicomSet (maps from P2's port group)
            if profile.id() == "nes.famicom" {
                field_map.insert(("player2", "microphone"), 16);
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
