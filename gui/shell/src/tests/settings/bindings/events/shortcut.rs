use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};

use super::{shortcut_action_for_key, shortcut_command_for_key};
use crate::{
    session::commands::SessionCommand, settings::defaults::seed::default_shared_settings,
};

#[test]
fn shortcuts_resolve_to_session_commands() {
    let settings = default_shared_settings();

    assert_eq!(
        shortcut_command_for_key(&settings, KeyboardKey::F5),
        Some(SessionCommand::SaveActiveSlotOrNew)
    );
}

#[test]
fn fullscreen_shortcut_is_exposed_as_action() {
    let settings = default_shared_settings();

    assert_eq!(
        shortcut_action_for_key(&settings, KeyboardKey::F11),
        Some(ShortcutAction::ToggleFullscreen)
    );
    assert_eq!(shortcut_command_for_key(&settings, KeyboardKey::F11), None);
}
