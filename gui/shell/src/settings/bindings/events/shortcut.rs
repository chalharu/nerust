use nerust_gui_settings::{
    input::{KeyboardKey, ShortcutAction},
    shared::DesktopSharedSettings,
};

use crate::session::commands::SessionCommand;

pub fn shortcut_command_for_key(
    settings: &DesktopSharedSettings,
    key: KeyboardKey,
) -> Option<SessionCommand> {
    shortcut_action_for_key(settings, key).and_then(shortcut_action_to_command)
}

pub fn shortcut_action_for_key(
    settings: &DesktopSharedSettings,
    key: KeyboardKey,
) -> Option<ShortcutAction> {
    settings
        .input
        .shortcuts
        .keyboard
        .iter()
        .find(|binding| binding.key == Some(key))
        .map(|binding| binding.action)
}

fn shortcut_action_to_command(action: ShortcutAction) -> Option<SessionCommand> {
    Some(match action {
        ShortcutAction::TogglePause => SessionCommand::TogglePause,
        ShortcutAction::Reset => SessionCommand::Reset,
        ShortcutAction::SaveActiveSlot => SessionCommand::SaveActiveSlotOrNew,
        ShortcutAction::SelectNextSlot => SessionCommand::SelectNextSlot,
        ShortcutAction::SelectPreviousSlot => SessionCommand::SelectPreviousSlot,
        ShortcutAction::LoadActiveSlot => SessionCommand::LoadActiveSlot,
        ShortcutAction::ToggleFullscreen => return None,
    })
}

#[cfg(test)]
#[path = "../../../tests/settings/bindings/events/shortcut.rs"]
mod tests;
