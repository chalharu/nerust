//! Input-source vocabulary shared by physical gamepads and virtual pads.
//! Position-based buttons (`South` is the bottom face button on every vendor
//! layout); backends normalize here so shells never branch on vendors.

use super::key::Key;

/// Maximum pads the port table (and the session order) tracks.
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
    Misc1,
    Paddle1,
    Paddle2,
    Paddle3,
    Paddle4,
}

impl StandardButton {
    /// Button count; sizes the normalized state arrays.
    pub const COUNT: usize = 20;

    /// Array index for normalized state storage.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Short UI label, same role as [`Key::label`].
    pub fn label(self) -> &'static str {
        match self {
            Self::South => "South",
            Self::East => "East",
            Self::West => "West",
            Self::North => "North",
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
            Self::Misc1 => "Misc",
            Self::Paddle1 => "P1",
            Self::Paddle2 => "P2",
            Self::Paddle3 => "P3",
            Self::Paddle4 => "P4",
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
            Self::LeftStick
            | Self::RightStick
            | Self::Misc1
            | Self::Paddle1
            | Self::Paddle2
            | Self::Paddle3
            | Self::Paddle4 => None,
        }
    }

    /// Reverse of [`Self::abstract_key`]: the position button driving one
    /// console-agnostic button in the fixed standard layout.
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

/// Android keycode to position button (single shared table).
/// Keycodes 104/105 (L2/R2) stay unmapped: triggers are analog-only and
/// arrive as motion axes, and `StandardButton` has no trigger buttons.
pub fn android_keycode_to_button(keycode: i32) -> Option<StandardButton> {
    const TABLE: [(i32, StandardButton); 15] = [
        (19, StandardButton::DpadUp),
        (20, StandardButton::DpadDown),
        (21, StandardButton::DpadLeft),
        (22, StandardButton::DpadRight),
        (96, StandardButton::South),
        (97, StandardButton::East),
        (99, StandardButton::West),
        (100, StandardButton::North),
        (102, StandardButton::LeftShoulder),
        (103, StandardButton::RightShoulder),
        (106, StandardButton::LeftStick),
        (107, StandardButton::RightStick),
        (108, StandardButton::Start),
        (109, StandardButton::Back),
        (110, StandardButton::Guide),
    ];
    TABLE
        .binary_search_by_key(&keycode, |(code, _)| *code)
        .ok()
        .map(|index| TABLE[index].1)
}

/// Analog axes: two sticks plus two triggers. Digital-only consoles sample
/// these through the backend-common threshold, never per-backend logic.
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

/// One normalized input event from any source. Absolute values: the session
/// OR-machine owns press state, sources never track it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceEvent {
    Button {
        button: StandardButton,
        pressed: bool,
    },
    Axis {
        axis: GamepadAxis,
        value: f32,
    },
}

/// Normalized pad snapshot: the virtual-backend seam and headless test entry.
/// Axes are post-deadzone (`-1.0..=1.0`, `0.0` neutral); the deadzone math
/// lives in `gamepad-db`, never per backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadState {
    pub buttons: [bool; StandardButton::COUNT],
    pub axes: [f32; GamepadAxis::COUNT],
}

impl Default for GamepadState {
    fn default() -> Self {
        Self {
            buttons: [false; StandardButton::COUNT],
            axes: [0.0; GamepadAxis::COUNT],
        }
    }
}

impl GamepadState {
    /// Fold one event into the snapshot.
    pub fn apply_event(&mut self, event: SourceEvent) {
        match event {
            SourceEvent::Button { button, pressed } => {
                self.buttons[button.index()] = pressed;
            }
            SourceEvent::Axis { axis, value } => {
                self.axes[axis.index()] = value;
            }
        }
    }

    /// Whether the button is held in the snapshot.
    pub fn is_pressed(self, button: StandardButton) -> bool {
        self.buttons[button.index()]
    }

    /// Post-deadzone axis value.
    pub fn axis(self, axis: GamepadAxis) -> f32 {
        self.axes[axis.index()]
    }
}

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
    fn button_positions_map_to_console_keys_without_guessing() {
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
        ] {
            assert_eq!(button.abstract_key(), Some(key));
        }
        for button in [
            StandardButton::LeftStick,
            StandardButton::RightStick,
            StandardButton::Misc1,
            StandardButton::Paddle1,
        ] {
            assert_eq!(button.abstract_key(), None);
        }
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
        assert_eq!(
            StandardButton::with_abstract(crate::AbstractKey::Button1),
            Some(StandardButton::South)
        );
        assert_eq!(StandardButton::with_abstract(A::Axis1X), None);
    }

    #[test]
    fn android_keycodes_cover_face_dpad_and_system_buttons() {
        for (keycode, button) in [
            (19, StandardButton::DpadUp),
            (20, StandardButton::DpadDown),
            (21, StandardButton::DpadLeft),
            (22, StandardButton::DpadRight),
            (96, StandardButton::South),
            (97, StandardButton::East),
            (99, StandardButton::West),
            (100, StandardButton::North),
            (102, StandardButton::LeftShoulder),
            (103, StandardButton::RightShoulder),
            (106, StandardButton::LeftStick),
            (107, StandardButton::RightStick),
            (108, StandardButton::Start),
            (109, StandardButton::Back),
            (110, StandardButton::Guide),
        ] {
            assert_eq!(android_keycode_to_button(keycode), Some(button));
        }
        // Triggers stay analog-only; unknown codes map to nothing.
        for keycode in [0, 98, 104, 105, 255] {
            assert_eq!(android_keycode_to_button(keycode), None);
        }
    }

    #[test]
    fn state_folds_events() {
        let mut state = GamepadState::default();
        assert!(!state.is_pressed(StandardButton::South));
        state.apply_event(SourceEvent::Button {
            button: StandardButton::South,
            pressed: true,
        });
        assert!(state.is_pressed(StandardButton::South));
        state.apply_event(SourceEvent::Axis {
            axis: GamepadAxis::LeftTrigger,
            value: 0.5,
        });
        assert_eq!(state.axis(GamepadAxis::LeftTrigger), 0.5);
        state.apply_event(SourceEvent::Button {
            button: StandardButton::South,
            pressed: false,
        });
        assert!(!state.is_pressed(StandardButton::South));
    }
}
