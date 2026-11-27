//! macOS motion via the GameController framework (`GCMotion`).
//!
//! gilrs exposes no IMU on any OS, but Apple controllers with motion
//! sensors (DualShock/DualSense class) publish gravity, user acceleration,
//! and rotation rate through `GCController.motion`. This polls that profile
//! once per frame and converts it to the shared cartridge sample.
//!
//! Scope notes:
//! - Getters only: no value-changed handlers are registered, so there is no
//!   runloop dependency; the frame loop calls this synchronously.
//! - Sensors that need it are activated on first sight
//!   (`sensorsRequireManualActivation`); activation persists on the
//!   controller, so later polls only read.
//! - Mapping mirrors the Android sensor: total acceleration (gravity plus
//!   user, the same signal `TYPE_ACCELEROMETER` delivers), planar x/y kept,
//!   clamped to the shared cartridge range. In-process transport needs no
//!   0.01g quantization; the session keeps latest-wins.
//!
//! Rumble lives next door in `macos_rumble` (same framework, haptics side).

use nerust_input_traits::{CARTRIDGE_ACCEL_MAX_G, GamepadMotion, MotionSource};
use objc2_game_controller::{GCController, GCMotion};

/// GameController motion profile drained once per frame. Stateless: every
/// poll re-reads the controller list, so hotplug needs no bookkeeping.
#[derive(Debug, Default)]
pub(crate) struct GcMotionSource;

impl GcMotionSource {
    fn sample_motion(motion: &GCMotion) -> Option<GamepadMotion> {
        // SAFETY: property getters on a live controller object; no
        // callbacks fire, no aliasing writes happen on this thread.
        unsafe {
            if motion.sensorsRequireManualActivation() && !motion.sensorsActive() {
                motion.setSensorsActive(true);
            }
            if !motion.hasGravityAndUserAcceleration() {
                return None;
            }
            let gravity = motion.gravity();
            let user = motion.userAcceleration();
            let gyro = if motion.hasRotationRate() {
                let rate = motion.rotationRate();
                [rate.x as f32, rate.y as f32, rate.z as f32]
            } else {
                [0.0; 3]
            };
            Some(GamepadMotion {
                gyro,
                accel: [
                    clamp_g(gravity.x + user.x),
                    clamp_g(gravity.y + user.y),
                    clamp_g(gravity.z + user.z),
                ],
            })
        }
    }

    fn sample() -> Option<GamepadMotion> {
        // SAFETY: snapshot getter; the retained array outlives this scope
        // and each controller is only read, never stored.
        unsafe {
            let controllers = GCController::controllers();
            let count = controllers.count();
            let mut index = 0;
            while index < count {
                let controller = controllers.objectAtIndex(index);
                if let Some(motion) = controller.motion()
                    && let Some(sample) = Self::sample_motion(&motion)
                {
                    return Some(sample);
                }
                index += 1;
            }
            None
        }
    }
}

impl MotionSource for GcMotionSource {
    fn poll_motion(&mut self) -> Option<GamepadMotion> {
        Self::sample()
    }
}

fn clamp_g(value: f64) -> f32 {
    (value as f32).clamp(-CARTRIDGE_ACCEL_MAX_G, CARTRIDGE_ACCEL_MAX_G)
}
