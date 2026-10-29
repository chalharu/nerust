//! Standalone blocking message dialog for boot-failure and crash paths.
//!
//! No event loop is running here (or it just died); pump a nested
//! `glib::MainLoop` until the user responds. Callers always log to stderr
//! too, so a missing display degrades to the log, never silence.

use gtk::{glib, prelude::*};

/// Show a native acknowledgment dialog and block until it is dismissed.
/// Returns immediately when no display is available.
pub fn show_message_dialog(dialog: &wakui_core::chrome::dialog::MessageDialog) {
    if !gtk::is_initialized() && gtk::init().is_err() {
        return;
    }
    let native = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        match dialog.level {
            wakui_core::chrome::dialog::MessageLevel::Info => gtk::MessageType::Info,
            wakui_core::chrome::dialog::MessageLevel::Warning => gtk::MessageType::Warning,
            wakui_core::chrome::dialog::MessageLevel::Error => gtk::MessageType::Error,
        },
        match dialog.buttons {
            wakui_core::chrome::dialog::MessageButtons::Ok => gtk::ButtonsType::Ok,
        },
        &dialog.description,
    );
    native.set_title(Some(dialog.title.as_str()));
    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    native.connect_response(move |dialog, _| {
        dialog.destroy();
        quit.quit();
    });
    native.present();
    main_loop.run();
}
