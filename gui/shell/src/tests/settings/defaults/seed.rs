use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::input::ShortcutAction;

use super::default_shared_settings;
use crate::test_support::TEST_CTRL_MIC;

#[test]
fn default_settings_seed_nes_bindings_and_system_settings() {
    let settings = default_shared_settings();

    assert!(settings.systems.contains_key(&SystemId::new("nes")));
    assert!(settings.input.systems.contains_key(&SystemId::new("nes")));
    assert!(
        settings
            .input
            .shortcuts
            .keyboard
            .iter()
            .any(|binding| binding.action == ShortcutAction::Reset && binding.key.is_none())
    );
    assert!(
        !settings
            .input
            .systems
            .get(&SystemId::new("nes"))
            .unwrap()
            .implicit_keyboard_profile()
            .unwrap()
            .bindings
            .iter()
            .any(|binding| binding.control.as_str() == TEST_CTRL_MIC.as_str())
    );
}
