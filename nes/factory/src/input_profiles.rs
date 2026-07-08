use nerust_input_traits::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, InputAssignments,
    InputPorts, InputResources, InputSplit, InputSystemFactory, PortSet, SlotInfo,
    CreateSplitError,
};
use nerust_nes_controller::input_buffer::NesInputBuffer;

#[derive(Debug)]
struct FamicomSet;

impl ControllerProfile for FamicomSet {
    fn id(&self) -> &'static str { "nes.famicom" }
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
        &[&["up", "down", "left", "right"]]
    }
}

static FAMICOM_SET: FamicomSet = FamicomSet;
static NES_CONTROLLERS: &[&'static dyn ControllerProfile] = &[&FAMICOM_SET];

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
                ("player1", Some("nes.famicom")),
                ("player2", None),
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
        let mut assigned_ports = std::collections::HashSet::<&str>::new();

        for (slot_id, ctrl_opt) in &assignments.slots {
            let ctrl_id = match ctrl_opt {
                Some(id) => *id,
                None => continue,
            };
            let slot: &str = slot_id;
            if ctrl_id != "nes.famicom" {
                return Err(CreateSplitError::ControllerNotFound {
                    controller: ctrl_id.to_string(),
                });
            }
            if !assigned_ports.insert(slot) {
                return Err(CreateSplitError::SlotConflict {
                    a: slot.to_string(),
                    b: slot.to_string(),
                });
            }
            // FamicomSet occupies {P1, P2}. Assign control groups by slot.
            let group_index = if slot == "player1" { 0 } else { 1 };
            let controls = FAMICOM_SET.port_groups()[group_index];
            for (fi, ci) in controls.iter().enumerate() {
                field_map.insert((slot, ci.id), group_index * 8 + fi);
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
