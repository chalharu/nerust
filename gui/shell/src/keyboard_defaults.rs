use nerust_gui_settings::input::KeyboardKey;
use nerust_input_traits::AbstractKey;

/// System-agnostic default keyboard binding for an abstract key.
/// Returns all sensible default keys (e.g. keyboard + numpad for D-pad).
pub fn default_keyboard_key(abstract_key: AbstractKey) -> Vec<KeyboardKey> {
    match abstract_key {
        AbstractKey::Button1 => vec![KeyboardKey::KeyZ],
        AbstractKey::Button2 => vec![KeyboardKey::KeyX],
        AbstractKey::Button3 => vec![KeyboardKey::KeyS],
        AbstractKey::Button4 => vec![KeyboardKey::KeyD],
        AbstractKey::Button5 => vec![KeyboardKey::KeyA],
        AbstractKey::Button6 => vec![KeyboardKey::KeyQ],
        AbstractKey::Button7 => vec![KeyboardKey::KeyW],
        AbstractKey::Button8 => vec![KeyboardKey::KeyE],
        AbstractKey::Select => vec![KeyboardKey::KeyC],
        AbstractKey::Start => vec![KeyboardKey::KeyV],
        AbstractKey::Guide => vec![],
        AbstractKey::DpadUp => vec![KeyboardKey::ArrowUp],
        AbstractKey::DpadDown => vec![KeyboardKey::ArrowDown],
        AbstractKey::DpadLeft => vec![KeyboardKey::ArrowLeft],
        AbstractKey::DpadRight => vec![KeyboardKey::ArrowRight],
        AbstractKey::Axis1X | AbstractKey::Axis1Y => vec![],
        AbstractKey::Axis2X | AbstractKey::Axis2Y => vec![],
    }
}
