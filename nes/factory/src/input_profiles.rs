use nerust_input_traits::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, InputAssignments,
    InputPorts, InputResources, InputSplit, InputSystemFactory, PortSet, SlotInfo,
    CreateSplitError,
};
use nerust_nes_controller::input_buffer::NesInputBuffer;

#[derive(Debug)]
struct StandardPad;

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
        &[&["up", "down", "left", "right"]]
    }
}

static STANDARD_PAD: StandardPad = StandardPad;
static NES_CONTROLLERS: &[&'static dyn ControllerProfile] = &[&STANDARD_PAD];

impl InputPorts for crate::NesFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[
            SlotInfo { id: "player1", label: "Player 1" },
            SlotInfo { id: "player2", label: "Player 2" },
        ];
        SLOTS
    }
    fn controllers(&self) -> &[&'static dyn ControllerProfile] { NES_CONTROLLERS }
}

impl InputSystemFactory for crate::NesFactory {
    fn default_assignments(&self) -> InputAssignments {
        InputAssignments {
            slots: vec![
                ("player1", Some("nes.standard_pad")),
                ("player2", Some("nes.standard_pad")),
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
            if ctrl_id != "nes.standard_pad" {
                return Err(CreateSplitError::ControllerNotFound {
                    controller: ctrl_id.to_string(),
                });
            }
            let slot_str: &str = slot_id;
            for ci in StandardPad.port_groups()[0] {
                field_map.insert((slot_str, ci.id), field_index);
                field_index += 1;
            }
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
