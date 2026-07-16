use keyboard_types::Code;
use nerust_gui_settings::input::{KeyboardBinding, PersistedControlId};
use nerust_input_traits::AbstractKey;

/// System-agnostic default keyboard binding for an abstract key.
/// Returns all sensible default keys (e.g. keyboard + numpad for D-pad).
pub fn default_keyboard_key(abstract_key: AbstractKey) -> Vec<Code> {
    match abstract_key {
        AbstractKey::Button1 => vec![Code::KeyZ],
        AbstractKey::Button2 => vec![Code::KeyX],
        AbstractKey::Button3 => vec![Code::KeyS],
        AbstractKey::Button4 => vec![Code::KeyD],
        AbstractKey::Button5 => vec![Code::KeyA],
        AbstractKey::Button6 => vec![Code::KeyQ],
        AbstractKey::Button7 => vec![Code::KeyW],
        AbstractKey::Button8 => vec![Code::KeyE],
        AbstractKey::Select => vec![Code::KeyC],
        AbstractKey::Start => vec![Code::KeyV],
        AbstractKey::Guide => vec![],
        AbstractKey::DpadUp => vec![Code::ArrowUp],
        AbstractKey::DpadDown => vec![Code::ArrowDown],
        AbstractKey::DpadLeft => vec![Code::ArrowLeft],
        AbstractKey::DpadRight => vec![Code::ArrowRight],
        AbstractKey::Axis1X | AbstractKey::Axis1Y => vec![],
        AbstractKey::Axis2X | AbstractKey::Axis2Y => vec![],
    }
}

/// Generate default NES keyboard bindings using abstract key mappings.
/// Matches the hardcoded defaults previously in seed.rs.
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
