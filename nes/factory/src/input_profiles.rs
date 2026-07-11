use std::rc::Rc;

use nerust_input_traits::{
    ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments, InputPorts,
    InputResources, InputSplit, InputSystemFactory, SlotInfo,
};
use nerust_nes_core::input_types::NesInputBuffer;

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
        use nerust_nes_core::controller::NES_PORTS;
        use std::sync::{Arc, Mutex};

        let mut field_map = std::collections::HashMap::new();
        for (idx, dev) in controllers.devices.iter().enumerate() {
            if idx >= NES_PORTS.len() {
                break;
            }
            for (s, ctrl, bit) in dev.field_map(&NES_PORTS[idx]) {
                field_map.insert((s, ctrl), bit);
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
