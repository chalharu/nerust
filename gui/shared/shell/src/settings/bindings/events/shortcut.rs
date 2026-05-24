use nerust_contract_settings::{
    desktop::DesktopSettings, input::KeyboardKey, shortcut::ShortcutAction,
};
use nerust_gui_session::commands::SessionCommand;

pub fn shortcut_command_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
) -> Option<SessionCommand> {
    shortcut_action_for_key(settings, key).and_then(shortcut_action_to_command)
}

pub fn shortcut_action_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
) -> Option<ShortcutAction> {
    settings
        .shortcuts
        .keyboard
        .iter()
        .find(|binding| binding.key == key)
        .map(|binding| binding.action)
}

fn shortcut_action_to_command(action: ShortcutAction) -> Option<SessionCommand> {
    Some(match action {
        ShortcutAction::TogglePause => SessionCommand::TogglePause,
        ShortcutAction::Reset => SessionCommand::Reset,
        ShortcutAction::SaveActiveSlotOrNew => SessionCommand::SaveActiveSlotOrNew,
        ShortcutAction::LoadActiveSlot => SessionCommand::LoadActiveSlot,
        ShortcutAction::SelectNextSlot => SessionCommand::SelectNextSlot,
        ShortcutAction::SelectPreviousSlot => SessionCommand::SelectPreviousSlot,
        ShortcutAction::ToggleFullscreen => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{shortcut_action_for_key, shortcut_command_for_key};
    use crate::settings::defaults::seed::default_desktop_settings;
    use nerust_contract_settings::{input::KeyboardKey, shortcut::ShortcutAction};
    use nerust_gui_session::commands::SessionCommand;

    #[test]
    fn shortcuts_resolve_to_session_commands() {
        let settings = default_desktop_settings();

        assert_eq!(
            shortcut_command_for_key(&settings, KeyboardKey::F5),
            Some(SessionCommand::SaveActiveSlotOrNew)
        );
    }

    #[test]
    fn fullscreen_shortcut_is_exposed_as_action() {
        let settings = default_desktop_settings();

        assert_eq!(
            shortcut_action_for_key(&settings, KeyboardKey::F11),
            Some(ShortcutAction::ToggleFullscreen)
        );
        assert_eq!(shortcut_command_for_key(&settings, KeyboardKey::F11), None);
    }
}
