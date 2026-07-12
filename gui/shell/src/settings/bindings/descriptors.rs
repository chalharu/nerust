use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::input::ShortcutAction;
use nerust_input_traits::{
    AttachmentId, ControlDescriptor, DigitalControlId, InputTopologyDescriptor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardBindingDescriptor {
    pub system: SystemId,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub control: DigitalControlId,
    pub control_label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBindingSectionDescriptor {
    pub system: SystemId,
    pub attachment: AttachmentId,
    pub attachment_label: &'static str,
    pub bindings: Vec<KeyboardBindingDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    pub action: ShortcutAction,
    pub label: &'static str,
}

const SHORTCUT_DESCRIPTORS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        action: ShortcutAction::TogglePause,
        label: "Toggle pause",
    },
    ShortcutDescriptor {
        action: ShortcutAction::SaveActiveSlot,
        label: "Save active slot",
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
        action: ShortcutAction::LoadActiveSlot,
        label: "Load active slot",
    },
    ShortcutDescriptor {
        action: ShortcutAction::ToggleFullscreen,
        label: "Toggle fullscreen",
    },
    ShortcutDescriptor {
        action: ShortcutAction::Reset,
        label: "Reset",
    },
];

pub fn keyboard_binding_descriptors(
    topology: &InputTopologyDescriptor,
) -> Vec<KeyboardBindingDescriptor> {
    // system ID is irrelevant for pure binding descriptors (sections carry the system)
    keyboard_binding_sections(topology, SystemId::new("nes"))
        .into_iter()
        .flat_map(|section| section.bindings)
        .collect()
}

pub fn keyboard_binding_sections(
    topology: &InputTopologyDescriptor,
    system: SystemId,
) -> Vec<KeyboardBindingSectionDescriptor> {
    topology
        .ports
        .iter()
        .flat_map(|port| port.attachments.iter())
        .filter_map(|attachment| {
            let device = topology.device(attachment.device)?;
            let bindings = device
                .controls
                .iter()
                .filter_map(|control| match control {
                    ControlDescriptor::Digital(control) => Some(KeyboardBindingDescriptor {
                        system,
                        attachment: attachment.id,
                        attachment_label: attachment.label,
                        control: control.id,
                        control_label: control.label,
                    }),
                    ControlDescriptor::Analog(_) => None,
                })
                .collect::<Vec<_>>();
            Some(KeyboardBindingSectionDescriptor {
                system,
                attachment: attachment.id,
                attachment_label: attachment.label,
                bindings,
            })
        })
        .collect()
}

pub fn shortcut_descriptors() -> &'static [ShortcutDescriptor] {
    SHORTCUT_DESCRIPTORS
}

#[cfg(test)]
#[path = "../../tests/settings/bindings/descriptors.rs"]
mod tests;
