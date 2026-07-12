use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::input::KeyboardKey;

use super::conflicting_keys;
use crate::{
    settings::defaults::seed::default_shared_settings, test_support::single_port_topology,
};

#[test]
fn detects_conflicts_across_controls_and_shortcuts() {
    let mut settings = default_shared_settings();
    settings
        .input
        .shortcuts
        .keyboard
        .iter_mut()
        .find(|binding| {
            matches!(
                binding.action,
                nerust_gui_settings::input::ShortcutAction::TogglePause
            )
        })
        .unwrap()
        .key = Some(KeyboardKey::KeyZ);

    let conflicts = conflicting_keys(&settings, &single_port_topology(), SystemId::new("nes"));
    assert!(conflicts.contains_key(&KeyboardKey::KeyZ));
}
