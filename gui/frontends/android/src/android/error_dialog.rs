//! Error dialog bridge: OSD error batches surface as a modal dialog,
//! one at a time. Dismissal returns via the typed frontend event.

use super::{bridge, messages::FrontendEvent};
use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str, sys::jobject};
use winit::platform::android::activity::AndroidApp;

/// Ask the Java side to show the modal error dialog. Fire-and-forget:
/// dismissal (or show failure) reports back through the event bridge.
pub(crate) fn request_show_error_dialog(app: &AndroidApp, message: &str) {
    let callback_app = app.clone();
    let message = message.to_string();
    app.run_on_java_main_thread(Box::new(move || {
        let result = unsafe { JavaVM::from_raw(callback_app.vm_as_ptr() as _) }
            .attach_current_thread(|env| {
                let activity_raw = callback_app.activity_as_ptr() as jobject;
                let activity = unsafe { JObject::from_raw(env, activity_raw) };
                let text = env.new_string(&message)?;
                env.call_method(
                    &activity,
                    jni_str!("showErrorDialog"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(text.as_ref())],
                )?;
                Ok::<_, jni::errors::Error>(())
            });
        if let Err(error) = result {
            log::error!("failed to show Android error dialog: {error:?}");
            // No Kotlin dismiss will arrive: synthesize one so the gate,
            // flag, and auto-pause do not wedge on a dead bridge. The
            // controller drops duplicates, so this stays idempotent.
            bridge::with_state(|state| {
                state
                    .frontend_events
                    .push_back(FrontendEvent::ErrorDialogDismissed)
            });
            bridge::wake();
        }
    }));
}
