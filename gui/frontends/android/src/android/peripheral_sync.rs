//! PeripheralSync: cartridge peripheral JNI pushes owned by one struct.
//! Owns last-pushed config/intensity and the generation dirty flag.

use nerust_gui_shell::session::SessionHandle;
use nerust_settings_core::model::local::RumbleTarget;
use winit::platform::android::activity::AndroidApp;

use super::bridge;

/// Last-pushed peripheral state plus a generation dirty flag.
/// Pushes only on change; suspended all-off state skips `host_peripherals()`.
pub(super) struct PeripheralSync {
    last_config: Option<PeripheralConfig>,
    last_intensity: Option<u8>,
    config_dirty: bool,
}

/// One cartridge-peripheral push: motion, rumble switch, strength, target.
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
        // Suspended all-off with a clean generation cannot have changed:
        // return before the `host_peripherals()` access.
        if !is_resumed
            && !self.config_dirty
            && self.last_config == Some(PeripheralConfig::all_off())
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

        let config = PeripheralConfig {
            motion: motion_active,
            rumble: settings.rumble_enabled,
            strength: settings.rumble_strength_percent.min(100),
            target: settings.rumble_target,
        };
        if self.config_dirty || self.last_config != Some(config) {
            configure_cartridge_peripherals(
                app,
                config.motion,
                config.rumble,
                config.strength,
                config.target,
            );
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
        self.last_config = Some(PeripheralConfig::all_off());
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
