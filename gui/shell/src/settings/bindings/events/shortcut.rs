use nerust_keyboard::Key;
use nerust_settings_core::model::{input::ShortcutAction, shared::DesktopSharedSettings};

#[cfg(test)]
use crate::session::commands::SessionCommand;

#[cfg(test)]
pub fn shortcut_command_for_key(
    settings: &DesktopSharedSettings,
    key: Key,
) -> Option<SessionCommand> {
    shortcut_action_for_key(settings, key).and_then(shortcut_action_to_command)
}

pub fn shortcut_action_for_key(
    settings: &DesktopSharedSettings,
    key: Key,
) -> Option<ShortcutAction> {
    settings
        .input
        .shortcuts
        .keyboard
        .iter()
        .find(|binding| binding.key == Some(key))
        .map(|binding| binding.action)
}

#[cfg(test)]
fn shortcut_action_to_command(action: ShortcutAction) -> Option<SessionCommand> {
    Some(match action {
        ShortcutAction::TogglePause => SessionCommand::TogglePause,
        ShortcutAction::Reset => SessionCommand::Reset,
        ShortcutAction::SaveActiveSlot => SessionCommand::SaveActiveSlotOrNew,
        ShortcutAction::SelectNextSlot => SessionCommand::SelectNextSlot,
        ShortcutAction::SelectPreviousSlot => SessionCommand::SelectPreviousSlot,
        ShortcutAction::LoadActiveSlot => SessionCommand::LoadActiveSlot,
        ShortcutAction::ToggleFullscreen | ShortcutAction::OpenDebugger => return None,
    })
}

#[cfg(test)]
mod tests {
    use nerust_keyboard::Key;
    use nerust_settings_core::model::input::ShortcutAction;

    use super::{shortcut_action_for_key, shortcut_command_for_key};
    use crate::{
        session::commands::SessionCommand, settings::defaults::seed::default_shared_settings,
    };

    #[test]
    fn shortcuts_resolve_to_session_commands() {
        let settings = default_shared_settings(&[]);

        assert_eq!(
            shortcut_command_for_key(&settings, Key::F5),
            Some(SessionCommand::SaveActiveSlotOrNew)
        );
    }

    #[test]
    fn fullscreen_shortcut_is_exposed_as_action() {
        let settings = default_shared_settings(&[]);

        assert_eq!(
            shortcut_action_for_key(&settings, Key::F11),
            Some(ShortcutAction::ToggleFullscreen)
        );
        assert_eq!(shortcut_command_for_key(&settings, Key::F11), None);
    }

    #[test]
    fn debugger_shortcut_resolves_through_schema() {
        let settings = default_shared_settings(&[]);

        assert_eq!(
            shortcut_action_for_key(&settings, Key::F4),
            Some(ShortcutAction::OpenDebugger)
        );
        // Window-opening is not a session command.
        assert_eq!(shortcut_command_for_key(&settings, Key::F4), None);
    }
}
