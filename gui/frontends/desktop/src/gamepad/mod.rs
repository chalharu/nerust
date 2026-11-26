//! Desktop gilrs reader: HID polls in, shared pad events out.
//!
//! macOS-first (P2'): gilrs is a macOS-only dependency so Linux builds stay
//! free of its libudev system requirement until the Linux backend (P2'')
//! lands. `cargo deny` graphs the Linux target, so the macOS-only edge adds
//! no deny churn now; the Linux phase re-runs deny with the system dep.
//! Default gilrs features stay on: disabling them trips a gilrs-core
//! unconditional `compile_error!`, and WGI is exactly what the later
//! Windows phase wants.
//!
//! Input-only phase: macOS reports no force feedback and ships no motion
//! producer, so there is no `RumbleSink` here yet. Rumble output and motion
//! producers are P3; the session read side (`rumble_intensity`) and the
//! motion entry (`apply_motion`) already exist for them.
//!
//! Conventions (review-pinned):
//! - Matches over gilrs `Button`/`Axis` are exhaustive with no wildcard arm,
//!   so a gilrs upgrade that adds a variant fails the macOS build loudly
//!   instead of silently parking the new input. `EventType` is
//!   `#[non_exhaustive]`, so its match ends with an explicit drop arm: new
//!   event kinds park as ignored until deliberately handled.
//! - The session owns deadzone, trigger edge, stick substitution, and repeat
//!   suppression. gilrs runs with its stock filters (conditional dpad
//!   synthesis plus jitter pre-clean); this layer never re-normalizes.
//! - Dpad axes are dropped on purpose: dpad state arrives as buttons
//!   (natively, or via gilrs' conditional axis synthesis), and driving both
//!   would double-feed the session.

use nerust_input_traits::{DeviceEvent, PollSource};

#[cfg(target_os = "macos")]
use gilrs::{Axis as GilrsAxis, Button as GilrsButton, EventType as GilrsEventType, Gilrs};
#[cfg(target_os = "macos")]
use nerust_input_traits::{GamepadAxis, GamepadEvent, StandardButton};

/// gilrs button to shared position button. `C`/`Z` are pre-standard extras
/// with no shared counterpart and `Unknown` stays silent instead of
/// guessing; both ride nowhere.
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
/// and drains it once per tick (thread-free, like the retired IOHID
/// reader); the dedicated reader thread arrives with the Linux hidraw
/// reader in P2'', which needs background polling while suspended.
#[cfg(target_os = "macos")]
pub(crate) struct GilrsPoller {
    gilrs: Gilrs,
}

#[cfg(target_os = "macos")]
impl GilrsPoller {
    /// `None` parks the backend as absent: every later poll stays empty
    /// instead of retrying noisily. Connect a pad and restart to pick it up.
    pub(crate) fn new() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => Some(Self { gilrs }),
            Err(error) => {
                log::warn!("gilrs unavailable, gamepads parked as absent: {error:?}");
                None
            }
        }
    }

    fn convert(&self, id: gilrs::GamepadId, event: GilrsEventType) -> Option<DeviceEvent> {
        let device = usize::from(id) as i32;
        let event = match event {
            GilrsEventType::Connected => GamepadEvent::Connected {
                descriptor: self.gilrs.gamepad(id).name().to_owned(),
            },
            GilrsEventType::Disconnected => GamepadEvent::Disconnected,
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

#[cfg(target_os = "macos")]
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

/// Poll-to-session fan-in shared by every desktop reader: drain the batch,
/// apply each event through the single session entry, and collect the
/// shortcut edges for the keyboard-identical tail. Generic over the source
/// so host tests pin it with `FakeSource` on every target, including ones
/// where gilrs never compiles.
pub(crate) fn drain<C>(
    source: &mut impl PollSource,
    apply: impl FnMut(DeviceEvent) -> Option<C>,
) -> Vec<C> {
    source.poll().into_iter().filter_map(apply).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_input_traits::{FakeSource, GamepadEvent, StandardButton};

    #[test]
    fn drain_preserves_poll_order_and_collects_cuts() {
        let mut source = FakeSource::new(vec![
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
        ]);
        let mut seen = Vec::new();
        let cuts = drain(&mut source, |event| {
            let cut = (event.device == 1).then_some("cut");
            seen.push(event);
            cut
        });
        assert_eq!(
            seen.iter().map(|event| event.device).collect::<Vec<_>>(),
            vec![0, 1, 0],
            "batch order is arrival order",
        );
        assert_eq!(cuts, vec!["cut"]);
        assert!(
            source.poll().is_empty(),
            "drain consumes the batch exactly once"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn mac_button_map_covers_every_variant() {
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
    #[cfg(target_os = "macos")]
    fn mac_axis_map_covers_every_variant() {
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
}
