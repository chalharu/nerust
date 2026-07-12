use nerust_core_traits::identity::SystemId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};

use super::{CaptureTarget, apply_capture_target, current_binding_key};
use crate::{
    settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    },
    test_support::{TEST_ATT_P1, TEST_CTRL_A},
};

fn snapshot() -> SettingsSnapshot {
    SettingsSnapshot {
        shared: default_shared_settings(),
        local: default_local_settings(),
        app_state: default_app_state(),
    }
}

#[test]
fn reads_existing_control_binding() {
    let snapshot = snapshot();

    assert_eq!(
        current_binding_key(
            &snapshot,
            &CaptureTarget::Binding {
                system: SystemId::new("nes"),
                attachment: TEST_ATT_P1.as_str().to_string(),
                control: TEST_CTRL_A.as_str().to_string(),
            }
        ),
        Some(KeyboardKey::KeyZ)
    );
}

#[test]
fn updates_existing_control_binding() {
    let mut snapshot = snapshot();
    let target = CaptureTarget::Binding {
        system: SystemId::new("nes"),
        attachment: TEST_ATT_P1.as_str().to_string(),
        control: TEST_CTRL_A.as_str().to_string(),
    };

    apply_capture_target(&mut snapshot, &target, Some(KeyboardKey::KeyA));

    assert_eq!(
        current_binding_key(&snapshot, &target),
        Some(KeyboardKey::KeyA)
    );
}

#[test]
fn clears_existing_shortcut_binding() {
    let mut snapshot = snapshot();
    let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);

    apply_capture_target(&mut snapshot, &target, None);

    assert_eq!(current_binding_key(&snapshot, &target), None);
}
