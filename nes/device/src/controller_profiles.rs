use nerust_input_traits::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, PortSet,
};

#[derive(Debug)]
pub struct StandardPadProfile;

impl ControllerProfile for StandardPadProfile {
    fn id(&self) -> &'static str { "nes.standard_pad" }
    fn label(&self) -> &'static str { "NES Standard Controller" }
    fn port_sets(&self) -> &[PortSet] {
        &[PortSet { ports: &["player1"] }, PortSet { ports: &["player2"] }]
    }
    fn port_groups(&self) -> &[&[ControlInfo]] {
        use AbstractKey::*;
        use ControlKind::*;
        static C: &[ControlInfo] = &[
            ControlInfo { id: "a", label: "A", kind: Digital, abstract_key: Some(Button1) },
            ControlInfo { id: "b", label: "B", kind: Digital, abstract_key: Some(Button2) },
            ControlInfo { id: "select", label: "Select", kind: Digital, abstract_key: Some(Select) },
            ControlInfo { id: "start", label: "Start", kind: Digital, abstract_key: Some(Start) },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(DpadRight) },
        ];
        static G: &[&[ControlInfo]] = &[C];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}

#[derive(Debug)]
pub struct FamicomSetProfile;

impl ControllerProfile for FamicomSetProfile {
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
            ControlInfo { id: "microphone", label: "Microphone", kind: Digital, abstract_key: None },
            ControlInfo { id: "up", label: "Up", kind: Digital, abstract_key: Some(DpadUp) },
            ControlInfo { id: "down", label: "Down", kind: Digital, abstract_key: Some(DpadDown) },
            ControlInfo { id: "left", label: "Left", kind: Digital, abstract_key: Some(DpadLeft) },
            ControlInfo { id: "right", label: "Right", kind: Digital, abstract_key: Some(DpadRight) },
        ];
        static G: &[&[ControlInfo]] = &[P1, P2];
        G
    }
    fn directional_ids(&self) -> &[&[&'static str; 4]] {
        &[&["up", "down", "left", "right"]]
    }
}

pub static STANDARD_PAD_PROFILE: StandardPadProfile = StandardPadProfile;
pub static FAMICOM_SET_PROFILE: FamicomSetProfile = FamicomSetProfile;
pub static NES_CONTROLLER_PROFILES: &[&'static dyn ControllerProfile] = &[&FAMICOM_SET_PROFILE, &STANDARD_PAD_PROFILE];
