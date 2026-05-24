use nerust_contract_settings::shortcut::ShortcutAction;
use nerust_input_nes::topology::ids::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{AttachmentId, DigitalControlId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NesKeyboardBindingDescriptor {
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub control: DigitalControlId,
    pub control_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    pub action: ShortcutAction,
    pub label: &'static str,
}

const NES_KEYBOARD_BINDINGS: &[NesKeyboardBindingDescriptor] = &[
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_A,
        control_label: "A",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_B,
        control_label: "B",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_SELECT,
        control_label: "Select",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_START,
        control_label: "Start",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_UP,
        control_label: "Up",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_DOWN,
        control_label: "Down",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_LEFT,
        control_label: "Left",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_ONE,
        attachment_label: "Player 1",
        control: NES_CONTROL_RIGHT,
        control_label: "Right",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_A,
        control_label: "A",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_B,
        control_label: "B",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_UP,
        control_label: "Up",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_DOWN,
        control_label: "Down",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_LEFT,
        control_label: "Left",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: NES_CONTROL_RIGHT,
        control_label: "Right",
    },
    NesKeyboardBindingDescriptor {
        attachment: NES_ATTACHMENT_PLAYER_TWO,
        attachment_label: "Player 2",
        control: FAMICOM_P2_CONTROL_MICROPHONE,
        control_label: "Microphone",
    },
];

const SHORTCUT_DESCRIPTORS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        action: ShortcutAction::TogglePause,
        label: "Toggle pause",
    },
    ShortcutDescriptor {
        action: ShortcutAction::Reset,
        label: "Reset",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SaveActiveSlotOrNew,
        label: "Save active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::LoadActiveSlot,
        label: "Load active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectNextSlot,
        label: "Select next slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SelectPreviousSlot,
        label: "Select previous slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::ToggleFullscreen,
        label: "Toggle fullscreen",
    },
];

pub fn keyboard_binding_descriptors() -> &'static [NesKeyboardBindingDescriptor] {
    NES_KEYBOARD_BINDINGS
}

pub fn shortcut_descriptors() -> &'static [ShortcutDescriptor] {
    SHORTCUT_DESCRIPTORS
}
