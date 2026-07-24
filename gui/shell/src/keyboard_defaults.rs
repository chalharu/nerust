use nerust_gui_settings::input::{KeyboardBinding, PersistedControlId};
use nerust_input_traits::AbstractKey;
use nerust_keyboard::Key;

/// System-agnostic default keyboard binding for an abstract key.
/// Returns all sensible default keys (e.g. keyboard + numpad for D-pad).
pub fn default_keyboard_key(abstract_key: AbstractKey) -> Vec<Key> {
    match abstract_key {
        AbstractKey::Button1 => vec![Key::KeyZ],
        AbstractKey::Button2 => vec![Key::KeyX],
        AbstractKey::Button3 => vec![Key::KeyS],
        AbstractKey::Button4 => vec![Key::KeyD],
        AbstractKey::Button5 => vec![Key::KeyA],
        AbstractKey::Button6 => vec![Key::KeyQ],
        AbstractKey::Button7 => vec![Key::KeyW],
        AbstractKey::Button8 => vec![Key::KeyE],
        AbstractKey::Select => vec![Key::KeyC],
        AbstractKey::Start => vec![Key::KeyV],
        AbstractKey::Guide => vec![],
        AbstractKey::DpadUp => vec![Key::ArrowUp],
        AbstractKey::DpadDown => vec![Key::ArrowDown],
        AbstractKey::DpadLeft => vec![Key::ArrowLeft],
        AbstractKey::DpadRight => vec![Key::ArrowRight],
        AbstractKey::Axis1X | AbstractKey::Axis1Y => vec![],
        AbstractKey::Axis2X | AbstractKey::Axis2Y => vec![],
    }
}

/// Generate NES keyboard bindings using abstract key mappings.
///
/// For new systems, add a corresponding `default_<system>_bindings()`
/// function that calls `default_keyboard_key()` with the system's
/// attachment and control IDs.
pub fn default_nes_bindings() -> Vec<KeyboardBinding> {
    use AbstractKey::*;
    let p1 = |control: &str, ak: AbstractKey| -> Vec<KeyboardBinding> {
        default_keyboard_key(ak)
            .into_iter()
            .map(|key| {
                KeyboardBinding::new(
                    "nes.attachment.player1",
                    PersistedControlId::digital(format!("nes.control.{control}")),
                    key,
                )
            })
            .collect()
    };
    let mut b = Vec::new();
    b.extend(p1("a", Button1));
    b.extend(p1("b", Button2));
    b.extend(p1("select", Select));
    b.extend(p1("start", Start));
    b.extend(p1("up", DpadUp));
    b.extend(p1("down", DpadDown));
    b.extend(p1("left", DpadLeft));
    b.extend(p1("right", DpadRight));
    b
}
