//! PeripheralSync: rumble/motion JNI pushes owned by one struct.
//! Owns last-pushed config/intensity and the generation dirty flag.

use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::{Capability, RumbleSink};
use nerust_settings_core::model::local::RumbleTarget;
use winit::platform::android::activity::AndroidApp;

use super::bridge;

/// Android vibrator output as a shared-contract sink: one logical
/// output, Kotlin routes to the active controller's vibrator internally.
/// Intensity is console-global (not per-pad); concurrent pads share it.
pub(super) struct AndroidRumbleSink {
    app: AndroidApp,
    last: Option<u8>,
}

impl AndroidRumbleSink {
    fn new(app: &AndroidApp) -> Self {
        Self {
            app: app.clone(),
            last: None,
        }
    }

    fn reseed(&mut self, app: &AndroidApp) {
        self.app = app.clone();
    }

    /// Collapse the dual-motor range to the single handset/controller
    /// scale. Rounding (not truncation) keeps the u8 round-trip exact,
    /// so a polled intensity re-pushes the identical byte.
    fn intensity(low: f32, high: f32) -> u8 {
        (low.max(high).clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn reset(&mut self) {
        self.last = None;
    }

    fn is_quiet(&self) -> bool {
        matches!(self.last, Some(0))
    }
}

impl RumbleSink for AndroidRumbleSink {
    fn set_rumble(&mut self, _device: i32, low: f32, high: f32) {
        let intensity = Self::intensity(low, high);
        if self.last == Some(intensity) {
            return;
        }
        self.last = Some(intensity);
        set_rumble(&self.app, intensity);
    }

    fn capability(&self, _device: i32) -> Capability {
        // Backend paths exist (vibrator JNI, accelerometer); enablement
        // stays settings-gated in `PeripheralSync::sync`.
        Capability {
            rumble: true,
            motion: true,
        }
    }
}

/// Last-pushed peripheral state plus a generation dirty flag.
/// Pushes only on change; suspended all-off state skips `host_peripherals()`.
pub(super) struct PeripheralSync {
    last_config: Option<PeripheralConfig>,
    sink: Option<AndroidRumbleSink>,
    config_dirty: bool,
}

/// One rumble/motion push: motion, rumble switch, strength, target.
/// Named fields so a reorder fails to compile here, not silently in Kotlin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeripheralConfig {
    motion: bool,
    rumble: bool,
    strength: u8,
    target: RumbleTarget,
}

impl PeripheralConfig {
    /// Suspended all-off state: the dirty gate skips re-pushing zeros.
    const fn all_off() -> Self {
        Self {
            motion: false,
            rumble: false,
            strength: 0,
            target: RumbleTarget::Auto,
        }
    }
}

impl PeripheralSync {
    pub(super) fn new() -> Self {
        Self {
            last_config: None,
            sink: None,
            config_dirty: true,
        }
    }

    /// Mark the config generation dirty: resume, settings apply, and ROM
    /// load paths call this so the next sync re-pushes unconditionally.
    pub(super) fn mark_dirty(&mut self) {
        self.config_dirty = true;
    }

    pub(super) fn sync(&mut self, app: &AndroidApp, session: &mut SessionHandle, is_resumed: bool) {
        // Suspended all-off with a clean generation cannot have changed:
        // return before the `host_peripherals()` access.
        if !is_resumed
            && !self.config_dirty
            && self.last_config == Some(PeripheralConfig::all_off())
            && self.sink.as_ref().is_some_and(|sink| sink.is_quiet())
        {
            return;
        }
        let handles = session.host_peripherals();
        let requested = handles
            .accelerometer
            .as_ref()
            .is_some_and(|handle| handle.demand().requested);
        let settings = &session.settings_snapshot().local.rumble_motion;
        let motion_active = is_resumed && requested && settings.motion_enabled;
        if !motion_active && let Some(accelerometer) = handles.accelerometer.as_ref() {
            accelerometer.clear();
        }
        let config = PeripheralConfig {
            motion: motion_active,
            rumble: settings.rumble_enabled,
            strength: settings.rumble_strength_percent.min(100),
            target: settings.rumble_target,
        };
        if self.config_dirty || self.last_config != Some(config) {
            configure_rumble_motion(
                app,
                config.motion,
                config.rumble,
                config.strength,
                config.target,
            );
            self.last_config = Some(config);
            self.reset_rumble_push();
        }

        let intensity = if is_resumed && settings.rumble_enabled {
            session.rumble_intensity()
        } else {
            0
        };
        let sink = self.sink.get_or_insert_with(|| AndroidRumbleSink::new(app));
        sink.reseed(app);
        let level = intensity as f32 / 255.0;
        sink.set_rumble(0, level, level);
        self.config_dirty = false;
    }

    fn reset_rumble_push(&mut self) {
        if let Some(sink) = self.sink.as_mut() {
            sink.reset();
        }
    }

    pub(super) fn stop(&mut self, app: &AndroidApp, session: &mut SessionHandle) {
        if let Some(accelerometer) = session.host_peripherals().accelerometer.as_ref() {
            accelerometer.clear();
        }
        configure_rumble_motion(app, false, false, 0, RumbleTarget::Auto);
        let sink = self.sink.get_or_insert_with(|| AndroidRumbleSink::new(app));
        sink.reseed(app);
        sink.reset();
        sink.set_rumble(0, 0.0, 0.0);
        // Settle the cache to all-off so the next suspended sync hits the
        // dirty gate instead of re-pushing zeros every pass.
        self.last_config = Some(PeripheralConfig::all_off());
        self.config_dirty = false;
    }
}

fn configure_rumble_motion(
    app: &AndroidApp,
    motion_enabled: bool,
    rumble_enabled: bool,
    rumble_strength_percent: u8,
    rumble_target: RumbleTarget,
) {
    let target = match rumble_target {
        RumbleTarget::Auto => "auto",
        RumbleTarget::Handset => "handset",
        RumbleTarget::Controller => "controller",
    }
    .to_string();
    bridge::post_to_activity(
        app,
        "configure Android rumble/motion",
        move |env, activity| {
            let target = env.new_string(&target)?;
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::CONFIGURE_RUMBLE_MOTION,
                &[
                    jni::objects::JValue::Bool(motion_enabled),
                    jni::objects::JValue::Bool(rumble_enabled),
                    jni::objects::JValue::Int(i32::from(rumble_strength_percent)),
                    jni::objects::JValue::Object(target.as_ref()),
                ],
            )?;
            Ok(())
        },
        None,
    );
}

fn set_rumble(app: &AndroidApp, intensity: u8) {
    bridge::post_to_activity(
        app,
        "update Android rumble",
        move |env, activity| {
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::SET_RUMBLE,
                &[jni::objects::JValue::Int(i32::from(intensity))],
            )?;
            Ok(())
        },
        None,
    );
}
