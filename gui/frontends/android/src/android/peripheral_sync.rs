//! PeripheralSync: cartridge peripheral JNI pushes owned by one struct.
//!
//! Split out of the `AndroidFrontend` god object. Owns the last-pushed
//! motion/rumble config + intensity and the generation dirty flag; the
//! frontend keeps wiring (session, app) and passes them per call so
//! ownership stays obvious.

use nerust_gui_shell::session::SessionHandle;
use nerust_settings_core::model::local::RumbleTarget;
use winit::platform::android::activity::AndroidApp;

use super::bridge;

/// Last-pushed peripheral state plus a generation dirty flag. The full
/// JNI push path (motion config + rumble intensity) runs only when the
/// generation changed or the cached push differs; while suspended with
/// settled all-off state the sync early-outs before touching
/// `host_peripherals()` at all.
pub(super) struct PeripheralSync {
    last_config: Option<(bool, bool, u8, RumbleTarget)>,
    last_intensity: Option<u8>,
    config_dirty: bool,
}

impl PeripheralSync {
    pub(super) fn new() -> Self {
        Self {
            last_config: None,
            last_intensity: None,
            config_dirty: true,
        }
    }

    /// Mark the config generation dirty: resume, settings apply, and ROM
    /// load paths call this so the next sync re-pushes unconditionally.
    pub(super) fn mark_dirty(&mut self) {
        self.config_dirty = true;
    }

    pub(super) fn sync(&mut self, app: &AndroidApp, session: &mut SessionHandle, is_resumed: bool) {
        // Generation dirty gate: suspended + settled all-off + clean
        // generation means nothing can have changed (no session tick
        // advances the rumble state while paused), so return before the
        // `host_peripherals()` access.
        if !is_resumed
            && !self.config_dirty
            && self.last_config == Some((false, false, 0, RumbleTarget::Auto))
            && self.last_intensity == Some(0)
        {
            return;
        }
        let handles = session.host_peripherals();
        let requested = handles
            .accelerometer
            .as_ref()
            .is_some_and(|handle| handle.demand().requested);
        let settings = &session.settings_snapshot().local.cartridge_peripherals;
        let motion_active = is_resumed && requested && settings.motion_enabled;
        if !motion_active && let Some(accelerometer) = handles.accelerometer.as_ref() {
            accelerometer.clear();
        }
        bridge::bind_accelerometer(
            motion_active
                .then(|| handles.accelerometer.as_ref().cloned())
                .flatten(),
        );

        let config = (
            motion_active,
            settings.rumble_enabled,
            settings.rumble_strength_percent.min(100),
            settings.rumble_target,
        );
        if self.config_dirty || self.last_config != Some(config) {
            configure_cartridge_peripherals(app, config.0, config.1, config.2, config.3);
            self.last_config = Some(config);
            self.last_rumble_intensity_reset();
        }

        let intensity = if is_resumed && settings.rumble_enabled {
            handles
                .rumble
                .as_ref()
                .map_or(0, |handle| handle.snapshot().state.intensity)
        } else {
            0
        };
        if self.last_intensity != Some(intensity) {
            set_cartridge_rumble(app, intensity);
            self.last_intensity = Some(intensity);
        }
        self.config_dirty = false;
    }

    fn last_rumble_intensity_reset(&mut self) {
        self.last_intensity = None;
    }

    pub(super) fn stop(&mut self, app: &AndroidApp, session: &mut SessionHandle) {
        if let Some(accelerometer) = session.host_peripherals().accelerometer.as_ref() {
            accelerometer.clear();
        }
        bridge::bind_accelerometer(None);
        configure_cartridge_peripherals(app, false, false, 0, RumbleTarget::Auto);
        set_cartridge_rumble(app, 0);
        // Settle the cache to all-off so the next suspended sync hits the
        // dirty gate instead of re-pushing zeros every pass.
        self.last_config = Some((false, false, 0, RumbleTarget::Auto));
        self.last_intensity = Some(0);
        self.config_dirty = false;
    }
}

fn configure_cartridge_peripherals(
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
    bridge::call_on_activity(
        app,
        "configure Android cartridge peripherals",
        move |env, activity| {
            let target = env.new_string(&target)?;
            env.call_method(
                &activity,
                jni::jni_str!("configureCartridgePeripherals"),
                jni::jni_sig!("(ZZILjava/lang/String;)V"),
                &[
                    jni::objects::JValue::Bool(motion_enabled),
                    jni::objects::JValue::Bool(rumble_enabled),
                    jni::objects::JValue::Int(i32::from(rumble_strength_percent)),
                    jni::objects::JValue::Object(target.as_ref()),
                ],
            )?;
            Ok(())
        },
    );
}

fn set_cartridge_rumble(app: &AndroidApp, intensity: u8) {
    bridge::call_on_activity(
        app,
        "update Android cartridge rumble",
        move |env, activity| {
            env.call_method(
                &activity,
                jni::jni_str!("setCartridgeRumble"),
                jni::jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(i32::from(intensity))],
            )?;
            Ok(())
        },
    );
}
