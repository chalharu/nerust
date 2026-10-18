//! Outermost crash containment: hook install + guarded entry.
//!
//! The hook only preserves the log (never touches UX, locks, or session
//! state: it can run on any thread, possibly lock-holding). Dialogs are the
//! frontend's job via `run_guarded`'s `on_crash`, which runs after unwind on
//! the calling thread. State is never reused after a catch: the process
//! exits, so `AssertUnwindSafe` cannot leak into a resumed session.

use std::{
    any::Any,
    backtrace::Backtrace,
    panic::{self, PanicHookInfo},
    sync::Once,
};

/// Install the process-wide panic hook (idempotent, chains previous).
///
/// Safe to call from every frontend; `log` facade only, backend untouched.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
            log::error!("{}", HookLines::new(info));
            previous(info);
        }));
    });
}

/// Structured crash data for dialogs: getters only, log shape via `Display`.
pub struct CrashReport {
    message: String,
    thread: String,
    backtrace: String,
}

impl CrashReport {
    /// What panicked (payload text or fallback).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Thread that unwound into `run_guarded` (always the caller thread).
    pub fn thread_name(&self) -> &str {
        &self.thread
    }

    /// Captured at the catch site (approximate, not the panic site).
    pub fn backtrace(&self) -> &str {
        &self.backtrace
    }

    fn from_payload(payload: &(dyn Any + Send)) -> Self {
        let thread = std::thread::current();
        Self {
            message: payload_message(payload),
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            backtrace: Backtrace::force_capture().to_string(),
        }
    }
}

impl std::fmt::Display for CrashReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "crashed on thread '{}': {}\nbacktrace:\n{}",
            self.thread, self.message, self.backtrace
        )
    }
}

/// Hook log lines (kept separate so both hook and report share one shape).
struct HookLines {
    thread: String,
    info: String,
    backtrace: String,
}

impl HookLines {
    fn new(info: &PanicHookInfo<'_>) -> Self {
        let thread = std::thread::current();
        Self {
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            info: info.to_string(),
            backtrace: Backtrace::force_capture().to_string(),
        }
    }
}

impl std::fmt::Display for HookLines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "panic on thread '{}': {}\nbacktrace:\n{}",
            self.thread, self.info, self.backtrace
        )
    }
}

fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Run `main` under the hook; on unwind log, offer `on_crash`, exit(101).
///
/// `on_crash` runs after unwind on the caller (UI) thread only; an
/// emu-thread panic never reaches it (typed `WorkerUnavailable` path
/// covers that). `on_crash` itself is catch-guarded so a failing dialog
/// cannot void the preserved log.
pub fn run_guarded<R>(main: impl FnOnce() -> R, on_crash: impl FnOnce(&CrashReport)) -> R {
    install();
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => value,
        Err(payload) => {
            let report = CrashReport::from_payload(payload.as_ref());
            log::error!("{report}");
            let _ = panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_crash(&report)));
            std::process::exit(101);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_message_covers_string_forms() {
        assert_eq!(payload_message(&"static"), "static");
        assert_eq!(payload_message(&"owned".to_owned()), "owned");
        assert_eq!(payload_message(&42_u32), "non-string panic payload");
    }

    #[test]
    fn report_display_names_thread_and_message() {
        let report = CrashReport::from_payload(&"boom");
        let text = report.to_string();
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains("crashed on thread"), "{text}");
        assert_eq!(report.message(), "boom");
        assert!(!report.thread_name().is_empty());
        assert!(!report.backtrace().is_empty());
    }

    #[test]
    fn hook_lines_share_thread_shape() {
        let text = std::panic::catch_unwind(|| {
            install();
            let info = std::panic::Location::caller();
            format!(
                "{}",
                HookLines {
                    thread: "probe".to_owned(),
                    info: format!("explicit panic at {info}"),
                    backtrace: "test-trace".to_owned(),
                }
            )
        })
        .expect("probe body must not panic");
        assert!(text.contains("panic on thread 'probe'"), "{text}");
    }
}
