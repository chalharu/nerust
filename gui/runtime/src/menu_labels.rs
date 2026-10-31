use nerust_settings_core::i18n::menu_label_with_shortcut;
use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding, shortcut_key_label};

/// Menu label with the action's *current* shortcut hint
/// (`Save Active Slot (F5)`), or the bare label when unbound.
/// Binding-driven so Settings rebinding never leaves a stale
/// hardcoded key in the menu. Shared by gtk/tao menu builders.
pub fn shortcut_hint(base: &str, shortcuts: &[ShortcutBinding], action: ShortcutAction) -> String {
    match shortcut_key_label(shortcuts, action) {
        Some(key) => menu_label_with_shortcut(base, key),
        None => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_keyboard::Key;

    fn binding(action: ShortcutAction, key: Option<Key>) -> ShortcutBinding {
        ShortcutBinding { action, key }
    }

    #[test]
    fn appends_bound_key() {
        let bindings = [binding(ShortcutAction::TogglePause, Some(Key::F5))];
        assert_eq!(
            shortcut_hint("Pause", &bindings, ShortcutAction::TogglePause),
            "Pause (F5)"
        );
    }

    #[test]
    fn bare_when_unbound() {
        let bindings = [binding(ShortcutAction::TogglePause, Some(Key::F5))];
        assert_eq!(
            shortcut_hint("Pause", &[], ShortcutAction::TogglePause),
            "Pause"
        );
        let unbound = [binding(ShortcutAction::TogglePause, None)];
        assert_eq!(
            shortcut_hint("Pause", &unbound, ShortcutAction::TogglePause),
            "Pause"
        );
        assert_eq!(
            shortcut_hint("Pause", &bindings, ShortcutAction::Reset),
            "Pause"
        );
    }
}
