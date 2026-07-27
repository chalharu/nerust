#![cfg(test)]

use nerust_core_traits::declare_system_id;
use nerust_gui_settings::{
    input::{IMPLICIT_PROFILE_ID, ShortcutAction, ShortcutBinding},
    shared::DesktopSharedSettings,
};

use nerust_keyboard::Key;

declare_system_id!(pub(crate) DummySystemId, "dummy");
declare_system_id!(pub(crate) DummyOtherSystemId, "other");

pub fn test_nes_defaults() -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings::default();
    settings.systems.insert(
        Box::new(DummySystemId),
        Box::new(nerust_nes_settings::NesSettings::default())
            as Box<dyn nerust_settings_traits::SystemSettings>,
    );
    let mut input = nerust_gui_settings::input::SystemInputSettings::default();
    input.implicit_keyboard_profile_mut().bindings =
        crate::keyboard_defaults::default_system_bindings("nes.attachment.player1", "nes.control");
    let _ = input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings
        .input
        .systems
        .insert(Box::new(DummySystemId), input);
    settings.input.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Key::F5),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(Key::F6),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(Key::F7),
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(Key::F8),
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(Key::F11),
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        },
    ];
    settings
}
