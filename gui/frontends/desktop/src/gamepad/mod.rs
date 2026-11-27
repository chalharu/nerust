//! Desktop gilrs reader: HID polls in, shared pad events out.
//!
//! Cross-platform by construction: gilrs abstracts evdev (Linux), WGI
//! (Windows), and IOHID (macOS) behind one event vocabulary, so this layer
//! carries no `target_os` gates. The only platform-specific remainder is
//! the Switch hidraw reader (Linux-only, P2''), which stays separate.
//!
//! Linux link note: building needs the libudev system package
//! (`libudev-dev`); runtime device access (`/dev/input/event*`) is a P2''
//! packaging follow-up alongside the hidraw reader. Default gilrs features
//! stay on: disabling them trips a gilrs-core unconditional
//! `compile_error!`, and WGI is exactly what Windows uses.
//!
//! Input-plus-peripherals phase: pads ride gilrs everywhere; macOS motion
//! rides the GameController profile and macOS rumble rides GameController
//! haptics (both official, macOS 11+). Linux/Windows output drivers and
//! the raw-HID Switch IMU reader plug into the same seams later; the
//! session read side (`rumble_intensity`) and the motion entry
//! (`apply_motion`) already exist for them.
//!
//! Conventions (review-pinned):
//! - Matches over gilrs `Button`/`Axis` are exhaustive with no wildcard arm,
//!   so a gilrs upgrade that adds a variant fails the build loudly instead
//!   of silently parking the new input. `EventType` is `#[non_exhaustive]`,
//!   so its match ends with an explicit drop arm: new event kinds park as
//!   ignored until deliberately handled.
//! - The session owns deadzone, trigger edge, stick substitution, and repeat
//!   suppression. gilrs runs with its stock filters (conditional dpad
//!   synthesis plus jitter pre-clean); this layer never re-normalizes.
//! - Dpad axes are dropped on purpose: dpad state arrives as buttons
//!   (natively, or via gilrs' conditional axis synthesis), and driving both
//!   would double-feed the session.

use gilrs::{
    Axis as GilrsAxis, Button as GilrsButton, EventType as GilrsEventType, GamepadId, Gilrs,
};
#[cfg(any(target_os = "macos", test))]
use nerust_input_traits::CARTRIDGE_ACCEL_MAX_G;
#[cfg(not(target_os = "macos"))]
use nerust_input_traits::Capability;
use nerust_input_traits::{
    DeviceEvent, GamepadAxis, GamepadEvent, GamepadMotion, MotionSource, PollSource, RumbleSink,
    StandardButton,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_motion;
#[cfg(target_os = "macos")]
mod macos_rumble;

/// gilrs button to shared position button. `C`/`Z` are pre-standard extras
/// with no shared counterpart and `Unknown` stays silent instead of
/// guessing; both ride nowhere.
pub(crate) fn map_button(button: GilrsButton) -> Option<StandardButton> {
    match button {
        GilrsButton::South => Some(StandardButton::South),
        GilrsButton::East => Some(StandardButton::East),
        GilrsButton::West => Some(StandardButton::West),
        GilrsButton::North => Some(StandardButton::North),
        GilrsButton::Select => Some(StandardButton::Back),
        GilrsButton::Mode => Some(StandardButton::Guide),
        GilrsButton::Start => Some(StandardButton::Start),
        GilrsButton::LeftThumb => Some(StandardButton::LeftStick),
        GilrsButton::RightThumb => Some(StandardButton::RightStick),
        // gilrs names the digital shoulders `LeftTrigger`/`RightTrigger`
        // and the analog pulls `LeftTrigger2`/`RightTrigger2`, matching the
        // SDL b4/b5 versus a3/a4 grouping the Android router assumes.
        GilrsButton::LeftTrigger => Some(StandardButton::LeftShoulder),
        GilrsButton::RightTrigger => Some(StandardButton::RightShoulder),
        GilrsButton::LeftTrigger2 => Some(StandardButton::LeftTrigger),
        GilrsButton::RightTrigger2 => Some(StandardButton::RightTrigger),
        GilrsButton::DPadUp => Some(StandardButton::DpadUp),
        GilrsButton::DPadDown => Some(StandardButton::DpadDown),
        GilrsButton::DPadLeft => Some(StandardButton::DpadLeft),
        GilrsButton::DPadRight => Some(StandardButton::DpadRight),
        GilrsButton::C | GilrsButton::Z | GilrsButton::Unknown => None,
    }
}

/// gilrs axis to shared axis. Dpad axes are dropped (buttons carry them);
/// `Unknown` stays silent.
pub(crate) fn map_axis(axis: GilrsAxis) -> Option<GamepadAxis> {
    match axis {
        GilrsAxis::LeftStickX => Some(GamepadAxis::LeftStickX),
        GilrsAxis::LeftStickY => Some(GamepadAxis::LeftStickY),
        GilrsAxis::RightStickX => Some(GamepadAxis::RightStickX),
        GilrsAxis::RightStickY => Some(GamepadAxis::RightStickY),
        GilrsAxis::LeftZ => Some(GamepadAxis::LeftTrigger),
        GilrsAxis::RightZ => Some(GamepadAxis::RightTrigger),
        GilrsAxis::DPadX | GilrsAxis::DPadY | GilrsAxis::Unknown => None,
    }
}

/// Frame-polled gilrs source. The UI frame loop owns the single instance
/// and drains it once per tick (thread-free). The dedicated reader thread
/// arrives with a future Linux reader that needs background polling while
/// suspended.
pub(crate) struct GilrsPoller {
    gilrs: Gilrs,
    /// USB vendor per frontend device id, recorded on connect for the
    /// session's label-layout lookup.
    vendors: std::collections::HashMap<i32, u16>,
    /// Connect edges since the last session sync. Drained once per tick,
    /// so the per-frame sync never allocates when nothing connected.
    pending: Vec<(i32, u16)>,
}

/// Gilrs attachment slot: probe once, then stay live or stay parked.
/// Retrying a failed backend init every frame would spam failing syscalls
/// and one log line per tick on machines with no readable pad.
pub(crate) enum GilrsSlot {
    Unprobed,
    Live(Box<GilrsPoller>),
    Dead,
}

impl GilrsSlot {
    pub(crate) fn poller(&mut self) -> Option<&mut GilrsPoller> {
        if matches!(self, Self::Unprobed) {
            *self = match GilrsPoller::new() {
                Some(poller) => Self::Live(Box::new(poller)),
                None => Self::Dead,
            };
        }
        match self {
            Self::Live(poller) => Some(poller),
            Self::Unprobed | Self::Dead => None,
        }
    }
}

impl GilrsPoller {
    /// `None` parks the backend as absent: every later poll stays empty
    /// instead of retrying noisily. Connect a pad and restart to pick it up.
    pub(crate) fn new() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => Some(Self {
                gilrs,
                vendors: std::collections::HashMap::new(),
                pending: Vec::new(),
            }),
            Err(error) => {
                log::warn!("gilrs unavailable, gamepads parked as absent: {error:?}");
                None
            }
        }
    }

    /// Connect edges since the last tick, drained by the session sync.
    pub(crate) fn take_new_vendors(&mut self) -> Vec<(i32, u16)> {
        std::mem::take(&mut self.pending)
    }

    fn convert(&mut self, id: GamepadId, event: GilrsEventType) -> Option<DeviceEvent> {
        let device = usize::from(id) as i32;
        let event = match event {
            GilrsEventType::Connected => {
                let gamepad = self.gilrs.gamepad(id);
                if let Some(vendor) = gamepad.vendor_id() {
                    self.vendors.insert(device, vendor);
                    self.pending.push((device, vendor));
                }
                GamepadEvent::Connected {
                    descriptor: gamepad.name().to_owned(),
                }
            }
            GilrsEventType::Disconnected => {
                self.vendors.remove(&device);
                GamepadEvent::Disconnected
            }
            GilrsEventType::ButtonPressed(button, _) => GamepadEvent::Button {
                button: map_button(button)?,
                pressed: true,
            },
            GilrsEventType::ButtonReleased(button, _) => GamepadEvent::Button {
                button: map_button(button)?,
                pressed: false,
            },
            // Only emitted with the Repeat filter, which stays uninstalled:
            // the session owns repeat suppression, so a repeated edge must
            // never reach it.
            GilrsEventType::ButtonRepeated(_, _) => return None,
            // Analog pulls surface trigger travel; digital shoulders are
            // already covered by the press/release edges above.
            GilrsEventType::ButtonChanged(GilrsButton::LeftTrigger2, value, _) => {
                GamepadEvent::Axis {
                    axis: GamepadAxis::LeftTrigger,
                    value,
                }
            }
            GilrsEventType::ButtonChanged(GilrsButton::RightTrigger2, value, _) => {
                GamepadEvent::Axis {
                    axis: GamepadAxis::RightTrigger,
                    value,
                }
            }
            GilrsEventType::ButtonChanged(_, _, _) => return None,
            GilrsEventType::AxisChanged(axis, value, _) => GamepadEvent::Axis {
                axis: map_axis(axis)?,
                value,
            },
            GilrsEventType::Dropped | GilrsEventType::ForceFeedbackEffectCompleted => return None,
            // `EventType` is non_exhaustive: future kinds park here until
            // deliberately handled. Button/Axis matches above stay
            // wildcard-free so input additions still break loudly.
            _ => return None,
        };
        Some(DeviceEvent { device, event })
    }
}

impl PollSource for GilrsPoller {
    fn poll(&mut self) -> Vec<DeviceEvent> {
        let mut out = Vec::new();
        while let Some(event) = self.gilrs.next_event() {
            if let Some(device_event) = self.convert(event.id, event.event) {
                out.push(device_event);
            }
        }
        out
    }
}

/// Platform motion producer: GameController profile on macOS, honest
/// absence everywhere else until the raw-HID reader lands.
pub(crate) fn motion_source() -> Box<dyn MotionSource> {
    #[cfg(target_os = "macos")]
    return Box::new(macos_motion::GcMotionSource);
    #[cfg(not(target_os = "macos"))]
    return Box::new(NullMotionSource);
}

/// Platform rumble sink: GameController haptics on macOS (official
/// CoreHaptics path, macOS 11+); honest absence everywhere else until
/// per-device HID output drivers land.
pub(crate) fn rumble_sink() -> Box<dyn RumbleSink> {
    #[cfg(target_os = "macos")]
    return Box::new(macos_rumble::GcRumbleSink::default());
    #[cfg(not(target_os = "macos"))]
    return Box::new(NullRumbleSink);
}

/// Null rumble sink: honest absence. Used where no output driver exists
/// yet (Linux/Windows await per-device HID output drivers). macOS drives
/// the real thing through GameController haptics; this seam stays for the
/// platforms still waiting, and `capability` tells settings UI the truth
/// in the meantime. macOS-only exclusion: the real sink makes this dead
/// code there, and dead platform code rots.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub(crate) struct NullRumbleSink;

#[cfg(not(target_os = "macos"))]
impl RumbleSink for NullRumbleSink {
    fn set_rumble(&mut self, _device: i32, _low: f32, _high: f32) {}
    fn capability(&self, _device: i32) -> Capability {
        Capability {
            rumble: false,
            motion: false,
        }
    }
}

/// Null motion source: used where no IMU producer exists. gilrs exposes
/// no IMU on any OS; macOS overrides this constructor with the
/// GameController motion profile, and the raw-HID Switch reader (P2'')
/// will do the same on Linux. Same macOS-only exclusion as above.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub(crate) struct NullMotionSource;

#[cfg(not(target_os = "macos"))]
impl MotionSource for NullMotionSource {
    fn poll_motion(&mut self) -> Option<GamepadMotion> {
        None
    }
}

/// Motion sample math, producer-agnostic and hardware-free: total
/// acceleration (gravity plus user, the same signal an accelerometer
/// delivers), planar components kept, clamped to the cartridge range.
/// Rotation passes through when the hardware exposes it.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn motion_sample(
    gravity: [f64; 3],
    user: [f64; 3],
    rotation: Option<[f64; 3]>,
) -> GamepadMotion {
    GamepadMotion {
        gyro: rotation
            .map(|rate| rate.map(|axis| axis as f32))
            .unwrap_or([0.0; 3]),
        accel: [
            clamp_g(gravity[0] + user[0]),
            clamp_g(gravity[1] + user[1]),
            clamp_g(gravity[2] + user[2]),
        ],
    }
}

#[cfg(any(target_os = "macos", test))]
fn clamp_g(value: f64) -> f32 {
    (value as f32).clamp(-CARTRIDGE_ACCEL_MAX_G, CARTRIDGE_ACCEL_MAX_G)
}

/// Motor pair collapse for the single-sink backend: strongest motor wins,
/// normalized to 0.0-1.0. Pure so the edge policy stays pinned without
/// haptics hardware.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn rumble_level(low: f32, high: f32) -> f32 {
    low.max(high).clamp(0.0, 1.0)
}

/// Console-global rumble edge tracker: collapses the 0-255 intensity to
/// one level for the single-sink backend and only calls down on change,
/// mirroring button edges. Owns the last sent level, so sessions never
/// thread edge state through return values. The sink address is fixed at
/// 0 (single-sink collapse, Android-identical); per-pad addressing changes
/// the call, not this shape.
#[derive(Debug, Default)]
pub(crate) struct RumbleEdge {
    last_sent: u8,
}

impl RumbleEdge {
    pub(crate) fn sync(&mut self, intensity: u8, sink: &mut dyn RumbleSink) {
        if intensity == self.last_sent {
            return;
        }
        let level = f32::from(intensity) / 255.0;
        sink.set_rumble(0, level, level);
        self.last_sent = intensity;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_input_traits::{FakeSink, GamepadEvent, StandardButton};

    #[test]
    fn poll_order_is_arrival_order() {
        let mut poller_events = vec![
            DeviceEvent {
                device: 0,
                event: GamepadEvent::Connected {
                    descriptor: "pad-a".to_string(),
                },
            },
            DeviceEvent {
                device: 1,
                event: GamepadEvent::Button {
                    button: StandardButton::South,
                    pressed: true,
                },
            },
            DeviceEvent {
                device: 0,
                event: GamepadEvent::Disconnected,
            },
        ];
        // Single drain consumes the batch exactly once, in arrival order.
        let batch = std::mem::take(&mut poller_events);
        assert_eq!(
            batch.iter().map(|event| event.device).collect::<Vec<_>>(),
            vec![0, 1, 0],
        );
        assert!(poller_events.is_empty());
    }

    #[test]
    fn button_map_covers_every_variant() {
        // Exhaustive by construction (no wildcard in map_button); this pins
        // the intended target of each variant instead of just its presence.
        use GilrsButton as B;
        let mapped = [
            (B::South, Some(StandardButton::South)),
            (B::East, Some(StandardButton::East)),
            (B::West, Some(StandardButton::West)),
            (B::North, Some(StandardButton::North)),
            (B::Select, Some(StandardButton::Back)),
            (B::Mode, Some(StandardButton::Guide)),
            (B::Start, Some(StandardButton::Start)),
            (B::LeftThumb, Some(StandardButton::LeftStick)),
            (B::RightThumb, Some(StandardButton::RightStick)),
            (B::LeftTrigger, Some(StandardButton::LeftShoulder)),
            (B::RightTrigger, Some(StandardButton::RightShoulder)),
            (B::LeftTrigger2, Some(StandardButton::LeftTrigger)),
            (B::RightTrigger2, Some(StandardButton::RightTrigger)),
            (B::DPadUp, Some(StandardButton::DpadUp)),
            (B::DPadDown, Some(StandardButton::DpadDown)),
            (B::DPadLeft, Some(StandardButton::DpadLeft)),
            (B::DPadRight, Some(StandardButton::DpadRight)),
            (B::C, None),
            (B::Z, None),
            (B::Unknown, None),
        ];
        for (button, expected) in mapped {
            assert_eq!(map_button(button), expected, "{button:?}");
        }
    }

    #[test]
    fn axis_map_covers_every_variant() {
        use GilrsAxis as A;
        let mapped = [
            (A::LeftStickX, Some(GamepadAxis::LeftStickX)),
            (A::LeftStickY, Some(GamepadAxis::LeftStickY)),
            (A::RightStickX, Some(GamepadAxis::RightStickX)),
            (A::RightStickY, Some(GamepadAxis::RightStickY)),
            (A::LeftZ, Some(GamepadAxis::LeftTrigger)),
            (A::RightZ, Some(GamepadAxis::RightTrigger)),
            (A::DPadX, None),
            (A::DPadY, None),
            (A::Unknown, None),
        ];
        for (axis, expected) in mapped {
            assert_eq!(map_axis(axis), expected, "{axis:?}");
        }
    }

    #[test]
    fn rumble_edge_sends_console_global_level_on_edges_only() {
        let mut edge = RumbleEdge::default();
        let mut sink = FakeSink::default();
        // Parked at zero: no call, no motor traffic per frame.
        edge.sync(0, &mut sink);
        assert!(sink.calls.is_empty());
        // Rising edge collapses FULL to one level on the single sink.
        edge.sync(255, &mut sink);
        assert_eq!(sink.calls, vec![(0, 1.0, 1.0)]);
        // Held level repeats silently.
        edge.sync(255, &mut sink);
        assert_eq!(sink.calls.len(), 1);
        // Unload/fall lands back at zero exactly once.
        edge.sync(0, &mut sink);
        assert_eq!(sink.calls, vec![(0, 1.0, 1.0), (0, 0.0, 0.0)]);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn null_backends_report_absence_honestly() {
        let mut sink = NullRumbleSink;
        sink.set_rumble(0, 1.0, 1.0);
        assert!(!sink.capability(0).rumble);
        assert!(!sink.capability(0).motion);
        let mut motion = NullMotionSource;
        assert_eq!(motion.poll_motion(), None);
    }

    #[test]
    fn motion_sample_sums_clamps_and_carries_gyro() {
        // Flat pad under a rightward push: total acceleration is the sum.
        let sample = motion_sample([0.0, 0.0, -1.0], [0.5, 0.0, 0.0], None);
        assert_eq!(sample.accel, [0.5, 0.0, -1.0]);
        assert_eq!(sample.gyro, [0.0; 3], "unexposed axes stay unknown");
        // MBC7 latches saturate: producers clamp, never the session.
        let sample = motion_sample([100.0, -100.0, 0.0], [0.0, 0.0, 0.0], Some([1.0, 2.0, 3.0]));
        assert_eq!(sample.accel, [8.0, -8.0, 0.0]);
        assert_eq!(sample.gyro, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn rumble_level_collapses_to_strongest_motor() {
        assert_eq!(rumble_level(0.0, 1.0), 1.0);
        assert_eq!(rumble_level(0.5, 0.25), 0.5);
        assert_eq!(rumble_level(-1.0, 2.0), 1.0, "out-of-range inputs clamp");
        assert_eq!(rumble_level(0.0, 0.0), 0.0);
    }
}
