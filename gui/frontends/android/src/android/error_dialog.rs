//! Error dialog bridge: OSD error batches surface as a modal dialog,
//! one at a time. Dismissal returns via the typed frontend event.

use super::menu;
use winit::platform::android::activity::AndroidApp;

/// Ask the Java side to show the modal error dialog. Fire-and-forget:
/// dismissal (or show failure) reports back through the event bridge.
pub(super) fn request_show_error_dialog(
    app: &AndroidApp,
    id: crate::error_dialog_controller::DialogRequestId,
    message: &str,
) {
    let message = message.to_string();
    super::bridge::post_to_activity(
        app,
        "show Android error dialog",
        move |env, activity| {
            let text = env.new_string(&message)?;
            super::bridge::call_bridge_method(
                env,
                &activity,
                &super::bridge::methods::SHOW_ERROR_DIALOG,
                &[
                    jni::objects::JValue::Long(id.0 as i64),
                    jni::objects::JValue::Object(text.as_ref()),
                ],
            )?;
            Ok(())
        },
        Some(Box::new(move || {
            // Synthesize the dismiss a dead bridge cannot deliver.
            // Duplicate-safe: the controller drops repeats.
            menu::publish_dialog_dismissed(id);
        })),
    );
}
