//! Input-source vocabulary shared by physical gamepads and virtual pads.
//! Position-based buttons (`South` is the bottom face button on every vendor
//! layout); backends normalize here so shells never branch on vendors.

use super::key::Key;

/// Maximum pads the port table (and the session order) tracks.
/// Product bound, not a hardware limit: consoles here have at most two
/// player ports, USB allows more. Four covers spares/shared pads with
/// fixed-size tables everywhere (settings schema, hot path, UI); a fifth
/// pad parks behind an explicit notice instead of vanishing silently.
pub const MAX_PADS: usize = 4;

/// Position-based standard pad button. Glyph names (`A`/`B`) transpose per
/// vendor (Nintendo swaps them); positions do not, so bindings stay portable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum StandardButton {
    South,
    East,
    West,
    North,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl StandardButton {
    /// Button count; sizes the normalized state arrays.
    pub const COUNT: usize = 15;

    /// Array index for normalized state storage.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// UI label: cap print for the four face buttons (Xbox-layout pads
    /// and Android `BUTTON_A` names put A at the bottom, so rows show the
    /// letter users can find), position label for the rest. Positions are
    /// the binding identity (vendors transpose caps); only the hint
    /// letter differs for Nintendo-layout users.
    pub fn label(self) -> &'static str {
        match self {
            Self::South => "A",
            Self::East => "B",
            Self::West => "X",
            Self::North => "Y",
            Self::Back => "Back",
            Self::Guide => "Guide",
            Self::Start => "Start",
            Self::LeftStick => "L3",
            Self::RightStick => "R3",
            Self::LeftShoulder => "LB",
            Self::RightShoulder => "RB",
            Self::DpadUp => "D-Up",
            Self::DpadDown => "D-Down",
            Self::DpadLeft => "D-Left",
            Self::DpadRight => "D-Right",
        }
    }

    /// Console-agnostic button for default binding resolution. Stick clicks,
    /// misc, and paddles have no console counterpart yet, so they stay
    /// unbound instead of guessing.
    pub fn abstract_key(self) -> Option<crate::AbstractKey> {
        use crate::AbstractKey as A;
        match self {
            Self::South => Some(A::Button1),
            Self::East => Some(A::Button2),
            Self::West => Some(A::Button3),
            Self::North => Some(A::Button4),
            Self::LeftShoulder => Some(A::Button5),
            Self::RightShoulder => Some(A::Button6),
            Self::Back => Some(A::Select),
            Self::Guide => Some(A::Guide),
            Self::Start => Some(A::Start),
            Self::DpadUp => Some(A::DpadUp),
            Self::DpadDown => Some(A::DpadDown),
            Self::DpadLeft => Some(A::DpadLeft),
            Self::DpadRight => Some(A::DpadRight),
            Self::LeftStick | Self::RightStick => None,
        }
    }

    /// Reverse of [`Self::abstract_key`]: the position button driving one
    /// console-agnostic button in the fixed standard layout. Direct match
    /// so the two never skew without a linear scan.
    pub fn with_abstract(abstract_key: crate::AbstractKey) -> Option<Self> {
        use crate::AbstractKey as A;
        match abstract_key {
            A::Button1 => Some(Self::South),
            A::Button2 => Some(Self::East),
            A::Button3 => Some(Self::West),
            A::Button4 => Some(Self::North),
            A::Button5 => Some(Self::LeftShoulder),
            A::Button6 => Some(Self::RightShoulder),
            A::Select => Some(Self::Back),
            A::Guide => Some(Self::Guide),
            A::Start => Some(Self::Start),
            A::DpadUp => Some(Self::DpadUp),
            A::DpadDown => Some(Self::DpadDown),
            A::DpadLeft => Some(Self::DpadLeft),
            A::DpadRight => Some(Self::DpadRight),
            _ => None,
        }
    }
}

/// Deadzone with rescale: inside the zone is `0.0`, the rim stays `1.0`.
/// `deadzone >= 1.0` parks everything at neutral instead of dividing by zero.
/// Single home for the math; backends never reimplement it.
pub fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    if deadzone >= 1.0 || value.abs() < deadzone {
        return 0.0;
    }
    (value - deadzone.copysign(value)) / (1.0 - deadzone)
}

/// Analog axes: two sticks plus two triggers. Digital-only consoles sample
/// these through the backend-common threshold, never per-backend logic.
/// Y axes are screen-down-positive on every backend (Android and winit
/// both report stick-up as negative Y), so substitute code reads `-Y` as
/// Up; no backend ever flips before this seam.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

impl GamepadAxis {
    /// Axis count; sizes the normalized state arrays.
    pub const COUNT: usize = 6;

    /// Array index for normalized state storage.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Console-agnostic axis for default resolution. Triggers have no
    /// console counterpart yet, so they stay unbound instead of guessing.
    pub fn abstract_key(self) -> Option<crate::AbstractKey> {
        use crate::AbstractKey as A;
        match self {
            Self::LeftStickX => Some(A::Axis1X),
            Self::LeftStickY => Some(A::Axis1Y),
            Self::RightStickX => Some(A::Axis2X),
            Self::RightStickY => Some(A::Axis2Y),
            Self::LeftTrigger | Self::RightTrigger => None,
        }
    }
}

/// Unified physical button: keyboard keys ride unchanged, pads use positions.
/// Two variants, closed: touch overlays emit `Pad` directly as virtual pads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceButtonId {
    Key(Key),
    Pad(StandardButton),
}

/// Stable source identity for port assignment. Touch overlays take a virtual
/// pad GUID, so no third variant is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceId {
    Keyboard(u32),
    Pad(PadGuid),
}

impl SourceId {
    /// The shared hardware keyboard. Keyboards ride one namespace;
    /// pads and touch take `Pad` with per-device GUIDs.
    pub const KEYBOARD: Self = Self::Keyboard(0);
}

/// Opaque 128-bit pad identity (SDL GUID bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PadGuid(pub [u8; 16]);

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn every_button_has_a_label_and_count_matches() {
        let mut count = 0;
        for button in StandardButton::iter() {
            assert!(!button.label().is_empty(), "{button:?} must have a label");
            assert_eq!(button.index(), count);
            count += 1;
        }
        assert_eq!(count, StandardButton::COUNT);
        let mut axes = 0;
        for axis in GamepadAxis::iter() {
            assert_eq!(axis.index(), axes);
            axes += 1;
        }
        assert_eq!(axes, GamepadAxis::COUNT);
    }

    #[test]
    fn face_buttons_show_cap_prints() {
        assert_eq!(
            [
                StandardButton::South.label(),
                StandardButton::East.label(),
                StandardButton::West.label(),
                StandardButton::North.label(),
            ],
            ["A", "B", "X", "Y"]
        );
    }

    #[test]
    fn sticks_map_to_console_axes_triggers_stay_unbound() {
        use crate::AbstractKey as A;
        for (axis, key) in [
            (GamepadAxis::LeftStickX, A::Axis1X),
            (GamepadAxis::LeftStickY, A::Axis1Y),
            (GamepadAxis::RightStickX, A::Axis2X),
            (GamepadAxis::RightStickY, A::Axis2Y),
        ] {
            assert_eq!(axis.abstract_key(), Some(key));
        }
        assert_eq!(GamepadAxis::LeftTrigger.abstract_key(), None);
        assert_eq!(GamepadAxis::RightTrigger.abstract_key(), None);
    }

    #[test]
    fn source_ids_cover_keyboard_and_pad() {
        assert_ne!(
            SourceId::Keyboard(0),
            SourceId::Pad(PadGuid([0; 16])),
            "keyboard and pad namespaces stay disjoint"
        );
    }

    #[test]
    fn abstract_round_trips_through_the_fixed_layout() {
        use crate::AbstractKey as A;
        for (button, key) in [
            (StandardButton::South, A::Button1),
            (StandardButton::East, A::Button2),
            (StandardButton::West, A::Button3),
            (StandardButton::North, A::Button4),
            (StandardButton::LeftShoulder, A::Button5),
            (StandardButton::RightShoulder, A::Button6),
            (StandardButton::Back, A::Select),
            (StandardButton::Guide, A::Guide),
            (StandardButton::Start, A::Start),
            (StandardButton::DpadUp, A::DpadUp),
            (StandardButton::DpadDown, A::DpadDown),
            (StandardButton::DpadLeft, A::DpadLeft),
            (StandardButton::DpadRight, A::DpadRight),
        ] {
            assert_eq!(button.abstract_key(), Some(key));
            assert_eq!(StandardButton::with_abstract(key), Some(button));
        }
        for button in [StandardButton::LeftStick, StandardButton::RightStick] {
            assert_eq!(button.abstract_key(), None);
        }
        assert_eq!(
            StandardButton::with_abstract(crate::AbstractKey::Button1),
            Some(StandardButton::South)
        );
        assert_eq!(StandardButton::with_abstract(A::Axis1X), None);
    }

    #[test]
    fn deadzone_parks_and_rescales() {
        assert_eq!(apply_deadzone(0.05, 0.1), 0.0);
        assert_eq!(apply_deadzone(-0.05, 0.1), 0.0);
        assert_eq!(apply_deadzone(1.0, 0.2), 1.0);
        assert_eq!(apply_deadzone(-1.0, 0.2), -1.0);
        assert!((apply_deadzone(0.6, 0.2) - 0.5).abs() < 1e-6);
        assert_eq!(apply_deadzone(0.9, 1.0), 0.0);
        assert_eq!(apply_deadzone(0.9, 2.0), 0.0);
    }
}
