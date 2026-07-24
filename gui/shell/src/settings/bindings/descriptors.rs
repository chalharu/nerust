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
    system: SystemId,
) -> Vec<KeyboardBindingDescriptor> {
    keyboard_binding_sections(topology, system)
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
mod tests {
    use nerust_core_traits::identity::SystemId;

    use super::{keyboard_binding_sections, shortcut_descriptors};
    use crate::test_support::{TEST_ATT_P1, TEST_ATT_P2, TEST_CTRL_MIC, dual_port_topology};

    #[test]
    fn topology_driven_sections_keep_player_boundaries() {
        let sections = keyboard_binding_sections(&dual_port_topology(), SystemId::new("nes"));

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].attachment, TEST_ATT_P1);
        assert_eq!(sections[1].attachment, TEST_ATT_P2);
        assert!(
            sections[1]
                .bindings
                .iter()
                .any(|binding| binding.control == TEST_CTRL_MIC)
        );
    }

    #[test]
    fn shortcuts_remain_stable() {
        assert!(shortcut_descriptors().iter().any(|descriptor| matches!(
            descriptor.action,
            nerust_gui_settings::input::ShortcutAction::ToggleFullscreen
        )));
    }
}
