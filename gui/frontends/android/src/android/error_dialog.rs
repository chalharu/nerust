//! Error dialog bridge: OSD error batches surface as a modal dialog,
//! one at a time. Dismissal returns via the menu string bridge.

use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str, refs::Global, sys::jobject};
use winit::platform::android::activity::AndroidApp;

/// Ask the Java side to show the modal error dialog. Fire-and-forget:
/// dismissal (or show failure) reports back through `onMenuAction`.
pub(crate) fn request_show_error_dialog(app: &AndroidApp, message: &str) {
    let callback_app = app.clone();
    let message = message.to_string();
    app.run_on_java_main_thread(Box::new(move || {
        let result = unsafe { JavaVM::from_raw(callback_app.vm_as_ptr() as _) }
            .attach_current_thread(|env| {
                let activity_raw = callback_app.activity_as_ptr() as jobject;
                let activity =
                    unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
                let text = env.new_string(&message)?;
                env.call_method(
                    activity.as_ref(),
                    jni_str!("showErrorDialog"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(text.as_ref())],
                )?;
                Ok::<_, jni::errors::Error>(())
            });
        if let Err(error) = result {
            log::error!("failed to show Android error dialog: {error:?}");
        }
    }));
}
