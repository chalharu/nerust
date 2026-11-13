//! Input-source vocabulary shared by physical gamepads and virtual pads.
//! Position-based buttons (`South` is the bottom face button on every vendor
//! layout); backends normalize here so shells never branch on vendors.

use super::key::Key;

/// Position-based standard pad button. Glyph names (`A`/`B`) transpose per
/// vendor (Nintendo swaps them); positions do not, so bindings stay portable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
}

/// Analog axes: two sticks plus two triggers. Digital-only consoles sample
/// these through the backend-common threshold, never per-backend logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

/// Opaque 128-bit pad identity (SDL GUID bytes). Virtual pads use reserved values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PadGuid(pub [u8; 16]);

impl PadGuid {
    /// All-zero GUID: never a real device.
    pub const NIL: Self = Self([0; 16]);
    /// Touch overlay virtual pad: shares port assignment with physical pads.
    pub const VIRTUAL_TOUCH: Self = Self([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    /// Raw GUID bytes.
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Dual-motor rumble levels, `0.0..=1.0` each. Backends clamp; this is a value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GamepadRumble {
    pub low: f32,
    pub high: f32,
}

impl GamepadRumble {
    /// Silent rumble: the neutral value for unsupported or idle pads.
    pub const OFF: Self = Self {
        low: 0.0,
        high: 0.0,
    };

    /// Motor levels before backend clamping.
    pub const fn new(low: f32, high: f32) -> Self {
        Self { low, high }
    }
}

/// Gyroscope (rad/s) and accelerometer (g) sample. Eventized so backends
/// never grow a second transport; consoles that ignore motion drop it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GamepadMotion {
    pub gyro: [f32; 3],
    pub accel: [f32; 3],
}

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
    Motion(GamepadMotion),
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
            SourceEvent::Motion(_) => {}
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

/// Device capability flags. Rumble-less pads report `has_rumble: false`
/// instead of growing a second trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capability {
    pub has_rumble: bool,
    pub has_motion: bool,
}

/// Event poll half: the only trait keyboards and touch overlays implement.
/// Backends drain into the caller-owned buffer, so polling never allocates.
pub trait PollSource: Send {
    /// Drain pending events into `out`.
    fn poll(&mut self, out: &mut Vec<SourceEvent>);
}

/// Identity half: stable id, display name, and capability flags.
pub trait SourceInfo {
    /// Stable id for port assignment and persistence.
    fn source_id(&self) -> SourceId;
    /// OS display name for device lists.
    fn name(&self) -> &str;
    /// Rumble/motion support flags.
    fn capability(&self) -> Capability;
}

/// Rumble output half: implemented only where the OS backend can emit.
/// Fire-and-forget; backends clamp levels to `0.0..=1.0`.
pub trait RumbleSink {
    /// Emit one rumble command.
    fn set_rumble(&mut self, rumble: GamepadRumble);
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
    fn source_ids_cover_keyboard_pad_and_virtual_touch() {
        assert_ne!(
            SourceId::Keyboard(0),
            SourceId::Pad(PadGuid::NIL),
            "keyboard and pad namespaces stay disjoint"
        );
        assert_ne!(PadGuid::NIL, PadGuid::VIRTUAL_TOUCH);
        assert_eq!(PadGuid::NIL.as_bytes(), [0; 16]);
    }

    #[test]
    fn state_folds_events_and_ignores_motion() {
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
        state.apply_event(SourceEvent::Motion(GamepadMotion::default()));
        assert!(state.is_pressed(StandardButton::South));
        state.apply_event(SourceEvent::Button {
            button: StandardButton::South,
            pressed: false,
        });
        assert!(!state.is_pressed(StandardButton::South));
    }

    #[test]
    fn traits_split_so_keyboards_need_poll_only() {
        struct Keys {
            pending: Vec<SourceEvent>,
        }
        impl PollSource for Keys {
            fn poll(&mut self, out: &mut Vec<SourceEvent>) {
                out.append(&mut self.pending);
            }
        }
        let mut keys = Keys {
            pending: vec![SourceEvent::Button {
                button: StandardButton::Start,
                pressed: true,
            }],
        };
        let mut out = Vec::new();
        keys.poll(&mut out);
        assert_eq!(out.len(), 1);
        keys.poll(&mut out);
        assert_eq!(out.len(), 1, "drained sources stay drained");
    }
}
