//! Error dialog bridge: OSD error batches surface as a modal dialog,
//! one at a time. Dismissal returns via the typed frontend event.

use super::menu;
use jni::{jni_sig, jni_str};
use winit::platform::android::activity::AndroidApp;

/// Ask the Java side to show the modal error dialog. Fire-and-forget:
/// dismissal (or show failure) reports back through the event bridge.
pub(crate) fn request_show_error_dialog(app: &AndroidApp, message: &str) {
    let message = message.to_string();
    super::bridge::call_on_activity_or(
        app,
        "show Android error dialog",
        move |env, activity| {
            let text = env.new_string(&message)?;
            env.call_method(
                &activity,
                jni_str!("showErrorDialog"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[jni::objects::JValue::Object(text.as_ref())],
            )?;
            Ok(())
        },
        || {
            // No Kotlin dismiss will arrive: synthesize one so the gate
            // and auto-pause do not wedge on a dead bridge. The
            // controller drops duplicates, so this stays idempotent.
            menu::publish_dialog_dismissed();
        },
    );
}
