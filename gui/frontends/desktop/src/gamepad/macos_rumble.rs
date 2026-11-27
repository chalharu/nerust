//! macOS rumble via GameController + CoreHaptics (official path, macOS 11+).
//!
//! Correction: an earlier revision claimed macOS had no official rumble
//! path because objc2-game-controller gates `createEngineWithLocality:` to
//! iOS/tvOS/visionOS. Apple's own `MacOSX.sdk` headers mark the whole
//! `GCDeviceHaptics` interface `API_AVAILABLE(macos(11.0), ...)` — SDL and
//! Chromium ship exactly this call on macOS — so the gate is binding
//! conservatism, not platform truth. The single unbound call goes through
//! `msg_send!`; everything else uses typed bindings.
//!
//! Shape (mirrors SDL/Chromium): one lazily-created engine on the default
//! locality, haptics-only, no auto-shutdown. Each intensity edge stops the
//! old player and starts a fresh infinite continuous pattern at the new
//! level; `sync_rumble` upstream already collapses repeats, so a call here
//! always means a new level. No stopped/reset handlers: a dropped engine
//! heals on the next nonzero edge, and rumble moments are edges by nature.
//! Single-sink collapse like Android: the first haptics-capable controller
//! drives the console-global level; `device` is accepted and ignored.

use objc2::{AnyThread, msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_core_haptics::{
    CHHapticEngine, CHHapticEvent, CHHapticEventParameter, CHHapticEventParameterIDHapticIntensity,
    CHHapticEventParameterIDHapticSharpness, CHHapticEventTypeHapticContinuous, CHHapticPattern,
    CHHapticPatternPlayer, CHHapticTimeImmediate,
};
use objc2_foundation::NSArray;
use objc2_game_controller::{
    GCController, GCDeviceHaptics, GCHapticDurationInfinite, GCHapticsLocalityDefault,
};

use nerust_input_traits::{Capability, RumbleSink};

/// Fixed sharpness for motor translation; game rumble carries no texture.
const SHARPNESS: f32 = 0.5;

struct LiveEngine {
    engine: Retained<CHHapticEngine>,
    player: Option<Retained<ProtocolObject<dyn CHHapticPatternPlayer>>>,
}

/// GameController haptics sink, created on first nonzero intensity.
/// Stateless until then: proportional cost, no startup failure modes.
#[derive(Default)]
pub(crate) struct GcRumbleSink {
    engine: Option<LiveEngine>,
    /// One-shot notice when the game rumbles with no haptics hardware.
    /// Edges can recur every moment; the log must not.
    warned_absent: bool,
}

impl GcRumbleSink {
    fn controller_haptics() -> Option<Retained<GCDeviceHaptics>> {
        // SAFETY: snapshot getter; the retained array outlives this scope
        // and each controller is only read, never stored.
        unsafe {
            let controllers = GCController::controllers();
            let count = controllers.count();
            let mut index = 0;
            while index < count {
                let controller = controllers.objectAtIndex(index);
                if let Some(haptics) = controller.haptics() {
                    return Some(haptics);
                }
                index += 1;
            }
            None
        }
    }

    fn start_engine() -> Option<LiveEngine> {
        let haptics = Self::controller_haptics()?;
        // SAFETY: the one call Apple's headers expose on macOS but the
        // bindings gate elsewhere; signature matches
        // `-createEngineWithLocality:` (nullable object return).
        let engine: Option<Retained<CHHapticEngine>> =
            unsafe { msg_send![&haptics, createEngineWithLocality: GCHapticsLocalityDefault] };
        let engine = engine?;
        // SAFETY: plain property setters plus fallible start on a live engine.
        unsafe {
            engine.setPlaysHapticsOnly(true);
            engine.setAutoShutdownEnabled(false);
            if engine.startAndReturnError().is_err() {
                return None;
            }
        }
        Some(LiveEngine {
            engine,
            player: None,
        })
    }

    fn stop_player(&mut self) {
        if let Some(live) = self.engine.as_mut()
            && let Some(player) = live.player.take()
        {
            // SAFETY: stop is idempotent; errors only mean "already quiet".
            unsafe {
                let _ = player.stopAtTime_error(CHHapticTimeImmediate);
            }
        }
    }

    fn play(
        engine: &CHHapticEngine,
        level: f32,
    ) -> Option<Retained<ProtocolObject<dyn CHHapticPatternPlayer>>> {
        // SAFETY: typed constructors with an infinite continuous event;
        // every fallible step returns None, never panics.
        unsafe {
            let intensity = CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                &CHHapticEventParameterIDHapticIntensity,
                level,
            );
            let sharpness = CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                &CHHapticEventParameterIDHapticSharpness,
                SHARPNESS,
            );
            let params = NSArray::from_retained_slice(&[intensity, sharpness]);
            let event = CHHapticEvent::initWithEventType_parameters_relativeTime_duration(
                CHHapticEvent::alloc(),
                &CHHapticEventTypeHapticContinuous,
                &params,
                0.0,
                GCHapticDurationInfinite as f64,
            );
            let events = NSArray::from_retained_slice(&[event]);
            let pattern = CHHapticPattern::initWithEvents_parameters_error(
                CHHapticPattern::alloc(),
                &events,
                &NSArray::from_slice(&[]),
            )
            .ok()?;
            let player = engine.createPlayerWithPattern_error(&pattern).ok()?;
            player.startAtTime_error(CHHapticTimeImmediate).ok()?;
            Some(player)
        }
    }
}

impl RumbleSink for GcRumbleSink {
    fn set_rumble(&mut self, _device: i32, low: f32, high: f32) {
        let level = low.max(high).clamp(0.0, 1.0);
        if level <= 0.0 {
            self.stop_player();
            return;
        }
        if self.engine.is_none() {
            self.engine = Self::start_engine();
            if self.engine.is_none() && !self.warned_absent {
                self.warned_absent = true;
                log::warn!("gamepad haptics unavailable; rumble parked until next edge");
            }
        }
        let player = match self.engine.as_ref() {
            Some(live) => Self::play(&live.engine, level),
            None => return,
        };
        self.stop_player();
        match player {
            Some(player) => {
                if let Some(live) = self.engine.as_mut() {
                    live.player = Some(player);
                }
            }
            None => {
                // Self-heal: drop the dead engine; the next edge recreates it.
                self.engine = None;
                log::warn!("gamepad haptics play failed; engine parked until next edge");
            }
        }
    }

    fn capability(&self, _device: i32) -> Capability {
        let rumble = Self::controller_haptics().is_some_and(|haptics| {
            // SAFETY: read-only count on a live object.
            (unsafe { haptics.supportedLocalities().count() }) > 0
        });
        Capability {
            rumble,
            motion: false,
        }
    }
}
