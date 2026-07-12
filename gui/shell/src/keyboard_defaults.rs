use nerust_gui_settings::input::{
    GamepadBinding, GamepadButton, GamepadButtonKind, KeyboardBinding, KeyboardKey,
    PersistedControlId,
};
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

/// Map AbstractKey to a default GamepadButton for the given player.
fn default_gamepad_button(abstract_key: AbstractKey) -> Option<GamepadButtonKind> {
    Some(match abstract_key {
        AbstractKey::Button1 => GamepadButtonKind::South,
        AbstractKey::Button2 => GamepadButtonKind::East,
        AbstractKey::Button3 => GamepadButtonKind::West,
        AbstractKey::Button4 => GamepadButtonKind::North,
        AbstractKey::Button5 => GamepadButtonKind::LeftTrigger,
        AbstractKey::Button6 => GamepadButtonKind::RightTrigger,
        AbstractKey::Button7 => GamepadButtonKind::LeftTrigger2,
        AbstractKey::Button8 => GamepadButtonKind::RightTrigger2,
        AbstractKey::Select => GamepadButtonKind::Select,
        AbstractKey::Start => GamepadButtonKind::Start,
        AbstractKey::Guide => GamepadButtonKind::Mode,
        AbstractKey::DpadUp => GamepadButtonKind::DpadUp,
        AbstractKey::DpadDown => GamepadButtonKind::DpadDown,
        AbstractKey::DpadLeft => GamepadButtonKind::DpadLeft,
        AbstractKey::DpadRight => GamepadButtonKind::DpadRight,
        AbstractKey::Axis1X | AbstractKey::Axis1Y | AbstractKey::Axis2X | AbstractKey::Axis2Y => {
            return None;
        }
    })
}

/// Generate default NES gamepad bindings for player 1 using abstract key mappings.
pub fn default_nes_gamepad_bindings() -> Vec<GamepadBinding> {
    use AbstractKey::*;
    let p1 = |control: &str, ak: AbstractKey| -> Vec<GamepadBinding> {
        let Some(button) = default_gamepad_button(ak) else {
            return Vec::new();
        };
        vec![GamepadBinding::new(
            "nes.attachment.player1",
            PersistedControlId::digital(format!("nes.control.{control}")),
            GamepadButton::new(0, button),
        )]
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
