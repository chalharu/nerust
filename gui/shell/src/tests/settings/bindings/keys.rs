use nerust_gui_settings::input::KeyboardKey;

use super::keyboard_key_label;

#[test]
fn keyboard_key_labels_are_human_readable() {
    assert_eq!(keyboard_key_label(KeyboardKey::ArrowUp), "Up");
}
