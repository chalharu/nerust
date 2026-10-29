//! `wakui_tao`: wakui platform frontend for iced/tao.
//!
//! Framework-only: depends on `wakui` plus the iced/tao toolkits.
//! Never on view definitions, sessions, or settings. Native frontends
//! render [`wakui_core::scene::control::ControlScreen`] snapshots through
//! [`scene::render`], adapt them to host windows through
//! [`scene::binding`], and map native input through [`input`].
//!
//! A third-party app builds UI with `wakui` plus this crate only:
//! see `examples/backend_smoke.rs`. The entry point is [`run`]; the
//! composition root injects a boxed [`wakui_core::app::boundary::FrontendApp`].

pub mod app;
pub mod backend;
pub mod event_sink;
pub mod input;
pub mod ports;

pub mod scene;

pub(crate) mod chrome;
pub(crate) mod flow;
pub(crate) mod host;

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod gtk_embed;

#[cfg(test)]
pub(crate) mod test_dialogs;
#[cfg(test)]
pub(crate) mod test_menu;
#[cfg(test)]
pub(crate) mod test_opener;
#[cfg(test)]
pub(crate) mod testkit;

pub use backend::{TaoBackend, register};

use wakui_core::app::boundary::Application;
use wakui_core::app::boundary::LaunchArguments;

/// Exit status for the binary wrapper: the library never terminates the process.
/// Crash containment and boot failure dialogs live in the composition root.
///
/// Launch arguments travel inside `app` (set by the composition root through
/// [`Application::builder`); they are opaque file operands, never
/// interpreted here. Following the platform file-open convention, the first
/// file is opened at startup; the rest is left to native open events.
pub fn run<K: Copy + Eq + std::hash::Hash + 'static, A: LaunchArguments>(
    app: Application<K, A>,
) -> i32 {
    run_inner(app);
    0
}

/// Standalone blocking message dialog for boot-failure and crash paths.
/// No event loop is running here (or it just died); `rfd` is an internal
/// detail of this flavor, never named by the composition root.
pub fn show_message_dialog(dialog: &wakui_core::chrome::dialog::MessageDialog) {
    rfd::MessageDialog::new()
        .set_title(&dialog.title)
        .set_description(&dialog.description)
        .set_level(match dialog.level {
            wakui_core::chrome::dialog::MessageLevel::Info => rfd::MessageLevel::Info,
            wakui_core::chrome::dialog::MessageLevel::Warning => rfd::MessageLevel::Warning,
            wakui_core::chrome::dialog::MessageLevel::Error => rfd::MessageLevel::Error,
        })
        .set_buttons(match dialog.buttons {
            wakui_core::chrome::dialog::MessageButtons::Ok => rfd::MessageButtons::Ok,
        })
        .show();
}

fn run_inner<K: Copy + Eq + std::hash::Hash + 'static, A: LaunchArguments>(app: Application<K, A>) {
    // Platform file-open convention: the first launch file opens at startup.
    // Later files arrive through native open events, if the platform has any.
    let launch_file = app.arguments().files().into_iter().next();
    let mut window = crate::app::Window::new(app);
    if let Some(path) = launch_file
        && !window.load_path(std::path::Path::new(&path))
    {
        log::info!("starting without a document (bad launch path)");
    }
    window.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_carries_launch_arguments_opaquely() {
        let app = Application::builder(Box::new(crate::testkit::FakeApp::new()))
            .argument("/tmp/launch.dat")
            .argument("--flag-like")
            .build();
        // Opaque: even flag-like strings ride along; the framework never
        // interprets them.
        assert_eq!(
            app.arguments(),
            &vec!["/tmp/launch.dat".to_string(), "--flag-like".to_string()]
        );
    }

    #[test]
    fn builder_arguments_replace_en_bloc() {
        let app = Application::builder(Box::new(crate::testkit::FakeApp::new()))
            .argument("/tmp/ignored.dat")
            .arguments(vec!["/tmp/launch.dat".to_string()])
            .build();
        assert_eq!(app.arguments(), &vec!["/tmp/launch.dat".to_string()]);
    }

    #[test]
    fn builder_defaults_to_no_arguments() {
        use wakui_core::app::boundary::LaunchArguments as _;
        let app = Application::new(Box::new(crate::testkit::FakeApp::new()));
        assert_eq!(app.arguments(), &());
        assert!(app.arguments().files().is_empty());
    }
}
