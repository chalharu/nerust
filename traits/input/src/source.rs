//! Input-source vocabulary shared by physical gamepads and virtual pads.
//! Position-based buttons (`South` is the bottom face button on every vendor
//! layout); backends normalize here so shells never branch on vendors.

use super::key::Key;

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
    /// Capture/share/create: the screenshot button vendors add beside
    /// Start (Switch capture, Xbox share, PS create arrive as
    /// `KEYCODE_BUTTON_C` on tested pads). No console counterpart (like
    /// stick clicks), so it stays abstract-free and serves shortcuts
    /// plus per-pad overrides only.
    Capture,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// Face-button naming by vendor. Positions stay canonical for bindings;
/// only the displayed glyph follows the pad in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadLayout {
    /// Sony (× ○ □ △): VID `0x054C`.
    Sony,
    /// Nintendo (B/A/Y/X diamond): VID `0x057E`.
    Nintendo,
    /// Xbox and everything else (A/B/X/Y diamond).
    Xbox,
}

impl PadLayout {
    /// Vendor to naming. Unknown or absent VIDs fall back to Xbox, the
    /// most common PC-pad diamond.
    pub fn for_vendor(vendor: Option<u16>) -> Self {
        match vendor {
            Some(0x054C) => Self::Sony,
            Some(0x057E) => Self::Nintendo,
            _ => Self::Xbox,
        }
    }
}

impl StandardButton {
    /// Console-agnostic button for default binding resolution. Stick clicks
    /// have no console counterpart yet, so they stay unbound instead of
    /// guessing. Single table so forward and reverse cannot skew.
    const LAYOUT: [(Self, crate::AbstractKey); 15] = [
        (Self::South, crate::AbstractKey::Button1),
        (Self::East, crate::AbstractKey::Button2),
        (Self::West, crate::AbstractKey::Button3),
        (Self::North, crate::AbstractKey::Button4),
        (Self::LeftShoulder, crate::AbstractKey::Button5),
        (Self::RightShoulder, crate::AbstractKey::Button6),
        (Self::LeftTrigger, crate::AbstractKey::Button7),
        (Self::RightTrigger, crate::AbstractKey::Button8),
        (Self::Back, crate::AbstractKey::Select),
        (Self::Guide, crate::AbstractKey::Guide),
        (Self::Start, crate::AbstractKey::Start),
        (Self::DpadUp, crate::AbstractKey::DpadUp),
        (Self::DpadDown, crate::AbstractKey::DpadDown),
        (Self::DpadLeft, crate::AbstractKey::DpadLeft),
        (Self::DpadRight, crate::AbstractKey::DpadRight),
    ];

    pub fn abstract_key(self) -> Option<crate::AbstractKey> {
        Self::LAYOUT
            .iter()
            .find(|(button, _)| *button == self)
            .map(|(_, key)| *key)
    }

    /// Reverse of [`Self::abstract_key`]: the position button driving one
    /// console-agnostic button in the fixed standard layout.
    pub fn with_abstract(abstract_key: crate::AbstractKey) -> Option<Self> {
        Self::LAYOUT
            .iter()
            .find(|(_, key)| *key == abstract_key)
            .map(|(button, _)| *button)
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
    /// Console-agnostic axis for default resolution.
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

/// One physical gamepad event, backend-agnostic. Poll readers (gilrs,
/// hidraw) and push edges (Android JNI) construct the same enum, so the
/// session owns exactly one entry point per direction.
/// No serde: session-internal events, never wire DTOs (the workspace
/// serde lacks `alloc`, and `Connected` carries an owned descriptor).
#[derive(Debug, Clone, PartialEq)]
pub enum GamepadEvent {
    Connected {
        descriptor: String,
    },
    Disconnected,
    Button {
        button: StandardButton,
        pressed: bool,
    },
    Axis {
        axis: GamepadAxis,
        value: f32,
    },
}

/// Controller motion sample: gyroscope and accelerometer, SI units
/// (rad/s, g). Producers fill what their hardware exposes; unexposed
/// axes stay zero, which means unknown, never measured stillness.
/// The session keeps the latest sample only.
/// No serde, same reason as [`GamepadEvent`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadMotion {
    pub gyro: [f32; 3],
    pub accel: [f32; 3],
}

/// Cartridge acceleration bound, g. Producers clamp to it; the session
/// never does (single home for the desktop bound; the Android bridge
/// enforces the same value from its generated contract).
pub const CARTRIDGE_ACCEL_MAX_G: f32 = 8.0;

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn every_button_has_a_label_and_count_matches() {
        let buttons: Vec<StandardButton> = StandardButton::iter().collect();
        assert_eq!(buttons.len(), 18);
        let axes: Vec<GamepadAxis> = GamepadAxis::iter().collect();
        assert_eq!(axes.len(), 6);
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
            (StandardButton::LeftTrigger, A::Button7),
            (StandardButton::RightTrigger, A::Button8),
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
        for button in [
            StandardButton::LeftStick,
            StandardButton::RightStick,
            StandardButton::Capture,
        ] {
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
