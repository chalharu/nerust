//! tao frontend: native window + iced (tiny-skia) UI.
//! [`event`] loop events, [`convert`] values, [`menu`], [`aux`], [`runtime`], [`app`].

mod app;
mod convert;
mod event;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod gtk_embed;
mod menu;
mod ports;
#[cfg(test)]
pub(crate) mod test_dialogs;
#[cfg(test)]
pub(crate) mod test_gpu;
#[cfg(test)]
pub(crate) mod test_menu;
#[cfg(test)]
pub(crate) mod test_opener;
#[cfg(test)]
pub(crate) mod test_session;
#[cfg(test)]
pub(crate) mod testkit;

pub(crate) mod aux;
pub(crate) mod runtime;
pub(crate) mod util;

use nerust_crash_guard::GuardOutcome;
use nerust_gui_shell::context::FrontendContext;
use nerust_run_options::RunOptions;

pub fn run(ctx: FrontendContext, options: RunOptions) {
    // A crash must never look like a clean exit; the dialog runs first.
    // Exhaustive: a future outcome must not silently pass through.
    match nerust_crash_guard::run_guarded_with(
        || run_inner(ctx, options),
        nerust_crash_guard::default_sink,
        nerust_crash_guard::capture_backtrace,
    ) {
        GuardOutcome::Completed(()) => {}
        GuardOutcome::Crashed(report) => {
            // Always English: crash copy must not depend on the settings
            // stack that may have just died. Stderr stays the log home.
            let dialog = crash_dialog_text(report.message());
            eprintln!("{dialog}");
            rfd::MessageDialog::new()
                .set_title(crash_title())
                .set_description(&dialog)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            std::process::exit(101); // Rust panic convention: a crash never exits 0.
        }
    }
}

/// Crash dialog copy: inline literals, never the catalog.
fn crash_title() -> &'static str {
    "nerust"
}

/// Always English: crash copy must not depend on the settings stack.
fn crash_dialog_text(message: &str) -> String {
    format!("The application crashed: {message}\n\nDetails were written to the log (stderr).")
}

fn run_inner(ctx: FrontendContext, options: RunOptions) {
    let mut window = match app::Window::new(ctx) {
        Ok(window) => window,
        Err(detail) => {
            log::error!("failed to initialise tao frontend: {detail}");
            rfd::MessageDialog::new()
                .set_title("nerust")
                .set_description(&detail)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            return;
        }
    };
    if let Some(path) = options.rom_path
        && !window.load_path(&path)
    {
        log::info!("starting without a ROM (bad --rom path)");
    }
    window.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_settings_core::i18n::{UiText, text};
    use nerust_settings_core::model::language::AppLanguage;

    #[test]
    fn crash_dialog_points_at_stderr_in_english() {
        let dialog = crash_dialog_text("boom");
        assert!(dialog.contains("boom"), "{dialog}");
        assert!(dialog.contains("The application crashed"), "{dialog}");
        assert!(dialog.contains("stderr"), "{dialog}");
        assert_eq!(crash_title(), "nerust");
        assert_eq!(
            text(AppLanguage::English, UiText::CrashTitle),
            crash_title()
        );
    }

    #[test]
    fn crash_dialog_is_english_without_language_input() {
        // No language param: the settings stack may have just died.
        let dialog = crash_dialog_text("boom");
        assert!(!dialog.contains("クラッシュ"), "{dialog}");
        assert!(dialog.contains("stderr"), "{dialog}");
        assert_eq!(
            text(AppLanguage::Japanese, UiText::CrashTitle),
            crash_title()
        );
    }
}
