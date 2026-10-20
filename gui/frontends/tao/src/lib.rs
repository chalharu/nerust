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

pub(crate) mod aux;
pub(crate) mod runtime;
pub(crate) mod util;

use nerust_crash_guard::{CRASH_EXIT_CODE, GuardOutcome};
use nerust_gui_shell::context::FrontendContext;
use nerust_run_options::RunOptions;
use nerust_settings_core::model::language::AppLanguage;

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
            // External portal dialog; stderr stays the log home. Inline literals only.
            let language = AppLanguage::SystemDefault;
            rfd::MessageDialog::new()
                .set_title(crash_title())
                .set_description(crash_dialog_text(language, report.message()))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            std::process::exit(CRASH_EXIT_CODE);
        }
    }
}

/// Crash dialog copy: inline literals, never the catalog.
fn crash_title() -> &'static str {
    "nerust"
}

fn crash_dialog_text(language: AppLanguage, message: &str) -> String {
    // English default; Japanese only when explicitly selected.
    let (body, details) = if matches!(language, AppLanguage::Japanese) {
        (
            "アプリケーションがクラッシュしました",
            "詳細はログ（stderr）に記録されました。",
        )
    } else {
        (
            "The application crashed",
            "Details were written to the log (stderr).",
        )
    };
    format!("{body}: {message}\n\n{details}")
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

    #[test]
    fn crash_dialog_points_at_stderr_in_english() {
        let dialog = crash_dialog_text(AppLanguage::English, "boom");
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
    fn crash_dialog_points_at_stderr_in_japanese() {
        let dialog = crash_dialog_text(AppLanguage::Japanese, "boom");
        assert!(dialog.contains("boom"), "{dialog}");
        assert!(
            dialog.contains("アプリケーションがクラッシュしました"),
            "{dialog}"
        );
        assert!(dialog.contains("stderr"), "{dialog}");
        assert_eq!(
            text(AppLanguage::Japanese, UiText::CrashTitle),
            crash_title()
        );
    }
}
