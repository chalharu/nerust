//! Backend contract for gamepad readers: polling, rumble output, and
//! capability reporting. Push edges (Android JNI) and poll readers
//! (gilrs, hidraw) meet here, so the session never depends on a backend.

use super::source::{GamepadEvent, GamepadMotion};

/// One polled batch item: the reader-local device id plus the event.
/// Device ids are frontend-assigned (`i32`, matching the session map);
/// the frontend owns their uniqueness across its readers.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceEvent {
    pub device: i32,
    pub event: GamepadEvent,
}

/// Poll-style gamepad source (gilrs, hidraw, virtual). Push edges do not
/// implement this; they construct `GamepadEvent` directly instead.
pub trait PollSource {
    fn poll(&mut self) -> Vec<DeviceEvent>;
}

/// Motion sample source, at most one IMU frame per poll. The session keeps
/// the latest sample only, so producers never queue: return `None` when no
/// new sample arrived since the last poll. gilrs exposes no IMU on any OS,
/// so desktop motion producers are raw-HID readers (Switch hidraw on
/// Linux); Android feeds its sensor through the menu lane instead.
pub trait MotionSource {
    fn poll_motion(&mut self) -> Option<GamepadMotion>;
}

/// Rumble output sink, one per backend. Intensity is normalized 0.0-1.0
/// per motor; backends without a motor report it via [`Capability`].
/// Single-sink backends (one console-global vibrator) collapse the
/// motors and may ignore `device`; per-pad backends address each device
/// independently.
pub trait RumbleSink {
    fn set_rumble(&mut self, device: i32, low: f32, high: f32);
    fn capability(&self, device: i32) -> Capability;
}

/// Per-device capability flags. Motion comes from the IMU reader only;
/// rumble is absent on backends without force feedback (e.g. macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub rumble: bool,
    pub motion: bool,
}

/// Test-only poll source: replays a scripted event list. Both frontend
/// test suites share it, so routing is pinned once, not per frontend.
#[derive(Debug, Default)]
pub struct FakeSource {
    events: Vec<DeviceEvent>,
}

impl FakeSource {
    pub fn new(events: Vec<DeviceEvent>) -> Self {
        Self { events }
    }
}

impl PollSource for FakeSource {
    fn poll(&mut self) -> Vec<DeviceEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Test-only rumble sink: records calls for assertions.
#[derive(Debug, Default)]
pub struct FakeSink {
    pub calls: Vec<(i32, f32, f32)>,
    pub capability: Option<Capability>,
}
impl RumbleSink for FakeSink {
    fn set_rumble(&mut self, device: i32, low: f32, high: f32) {
        self.calls.push((device, low, high));
    }

    fn capability(&self, _device: i32) -> Capability {
        self.capability.unwrap_or(Capability {
            rumble: false,
            motion: false,
        })
    }
}

/// Test-only motion source: replays one scripted sample per poll, then
/// parks at `None`. Pins the poll-to-apply seam without hardware.
#[derive(Debug, Default)]
pub struct FakeMotionSource {
    samples: Vec<GamepadMotion>,
}

impl FakeMotionSource {
    pub fn new(samples: Vec<GamepadMotion>) -> Self {
        Self { samples }
    }
}

impl MotionSource for FakeMotionSource {
    fn poll_motion(&mut self) -> Option<GamepadMotion> {
        if self.samples.is_empty() {
            None
        } else {
            Some(self.samples.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{GamepadAxis, StandardButton};

    #[test]
    fn fake_source_drains_in_order() {
        let mut source = FakeSource::new(vec![
            DeviceEvent {
                device: 7,
                event: GamepadEvent::Connected {
                    descriptor: "pad:a".to_string(),
                },
            },
            DeviceEvent {
                device: 7,
                event: GamepadEvent::Button {
                    button: StandardButton::South,
                    pressed: true,
                },
            },
            DeviceEvent {
                device: 7,
                event: GamepadEvent::Axis {
                    axis: GamepadAxis::LeftTrigger,
                    value: 0.5,
                },
            },
        ]);
        let batch = source.poll();
        assert_eq!(batch.len(), 3);
        assert!(source.poll().is_empty(), "drained source stays empty");
    }

    #[test]
    fn fake_sink_records_calls() {
        let mut sink = FakeSink {
            calls: Vec::new(),
            capability: Some(Capability {
                rumble: true,
                motion: false,
            }),
        };
        sink.set_rumble(7, 0.0, 1.0);
        assert_eq!(sink.calls, vec![(7, 0.0, 1.0)]);
        assert!(sink.capability(7).rumble);
        assert!(!sink.capability(7).motion);
    }

    #[test]
    fn fake_motion_source_replays_one_sample_per_poll() {
        use crate::source::GamepadMotion;
        let sample = |x: f32| GamepadMotion {
            gyro: [0.0; 3],
            accel: [x, 0.0, 0.0],
        };
        let mut source = FakeMotionSource::new(vec![sample(1.0), sample(-1.0)]);
        assert_eq!(source.poll_motion().map(|m| m.accel[0]), Some(1.0));
        assert_eq!(source.poll_motion().map(|m| m.accel[0]), Some(-1.0));
        assert_eq!(source.poll_motion(), None, "scripted samples run out");
    }
}
