//! ChromePort: overlay and haptic pushes to the Kotlin chrome.
//! All JNI crossings for the touch overlay route through here.

use nerust_core_traits::touch::TouchPoint;
use winit::platform::android::activity::AndroidApp;

use super::bridge;

/// Push the resolved zones plus the joystick home; Kotlin draws only these.
pub(super) fn configure_overlay(
    app: &AndroidApp,
    settings: &nerust_settings_core::model::local::TouchOverlaySettings,
    zones: String,
    joystick: String,
) {
    let visibility = match settings.visibility {
        nerust_settings_core::model::local::TouchOverlayVisibility::Always => "always",
        nerust_settings_core::model::local::TouchOverlayVisibility::Auto => "auto",
        nerust_settings_core::model::local::TouchOverlayVisibility::Hidden => "hidden",
    }
    .to_string();
    let opacity = i32::from(settings.opacity_percent);
    let haptics = settings.haptics;
    // Owned buffers move into the closure; the JNI hop copies nothing twice.
    bridge::post_to_activity(
        app,
        "configure Android controls overlay",
        move |env, activity| {
            let visibility = env.new_string(&visibility)?;
            let zones = env.new_string(&zones)?;
            let joystick = env.new_string(&joystick)?;
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::CONFIGURE_CONTROLS_OVERLAY,
                &[
                    jni::objects::JValue::Object(visibility.as_ref()),
                    jni::objects::JValue::Int(opacity),
                    jni::objects::JValue::Bool(haptics),
                    jni::objects::JValue::Object(zones.as_ref()),
                    jni::objects::JValue::Object(joystick.as_ref()),
                ],
            )?;
            Ok(())
        },
        None,
    );
}

/// Single haptic tick for a control press.
pub(super) fn haptic(app: &AndroidApp) {
    bridge::post_to_activity(
        app,
        "play Android control haptic",
        |env, activity| {
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::PERFORM_CONTROL_HAPTIC,
                &[],
            )?;
            Ok(())
        },
        None,
    );
}

/// Floating-joystick visual: active flag plus center and knob.
/// Callers quantize first; repeats never cross JNI.
pub(super) fn dpad_visual(app: &AndroidApp, active: bool, center: TouchPoint, knob: TouchPoint) {
    bridge::post_to_activity(
        app,
        "update Android dpad visual",
        move |env, activity| {
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::UPDATE_FLOATING_DPAD,
                &[
                    jni::objects::JValue::Bool(active),
                    jni::objects::JValue::Float(center.x),
                    jni::objects::JValue::Float(center.y),
                    jni::objects::JValue::Float(knob.x),
                    jni::objects::JValue::Float(knob.y),
                ],
            )?;
            Ok(())
        },
        None,
    );
}
