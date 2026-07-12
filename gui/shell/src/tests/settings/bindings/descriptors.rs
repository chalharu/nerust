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
