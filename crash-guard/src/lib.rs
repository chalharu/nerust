//! Outermost crash containment: hook install + guarded entry.
//!
//! The hook only preserves the log (never touches UX, locks, or session
//! state: it can run on any thread, possibly lock-holding). Dialogs are the
//! frontend's job via `run_guarded`'s `on_crash`, which runs after unwind on
//! the calling thread; the returned [`GuardOutcome`] lets the host
//! terminate (a crash must never look like a clean exit). Captured state
//! is never reused after a catch: `main` is moved into the guard, so
//! `AssertUnwindSafe` cannot leak into a resumed session.

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
            let record = PanicRecord::capture(info.to_string());
            log::error!(
                "panic on thread '{}': {}\nbacktrace:\n{}",
                record.thread,
                record.detail,
                record.backtrace
            );
            previous(info);
        }));
    });
}

/// Crash containment exit code, matching the Rust panic convention
/// so tooling can identify crashes automatically. Hosts exit with
/// this on [`GuardOutcome::Crashed`].
pub const CRASH_EXIT_CODE: i32 = 101;

/// Guard outcome: normal completion or an unwound crash. The crash
/// path is already logged (hook plus report) and `on_crash` already
/// ran; the host decides termination. `#[must_use]` keeps a crash
/// from silently passing as a clean exit.
#[must_use]
#[derive(Debug)]
pub enum GuardOutcome<R> {
    Completed(R),
    Crashed(CrashReport),
}

/// Structured crash data for dialogs: getters only, log shape via `Display`.
#[derive(Debug)]
pub struct CrashReport {
    record: PanicRecord,
}

impl CrashReport {
    /// What panicked (payload text or fallback).
    pub fn message(&self) -> &str {
        &self.record.detail
    }

    /// Thread that unwound into `run_guarded` (always the caller thread).
    pub fn thread_name(&self) -> &str {
        &self.record.thread
    }

    /// Captured at the catch site (approximate, not the panic site).
    pub fn backtrace(&self) -> &str {
        &self.record.backtrace
    }

    fn from_payload(payload: &(dyn Any + Send)) -> Self {
        Self {
            record: PanicRecord::capture(payload_message(payload)),
        }
    }
}

impl std::fmt::Display for CrashReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "crashed on thread '{}': {}\nbacktrace:\n{}",
            self.record.thread, self.record.detail, self.record.backtrace
        )
    }
}

/// Shared capture for hook and report: thread plus detail text plus
/// backtrace. Both log shapes draw from these parts instead of
/// duplicating the capture logic.
#[derive(Debug)]
struct PanicRecord {
    thread: String,
    detail: String,
    backtrace: String,
}

impl PanicRecord {
    fn capture(detail: String) -> Self {
        let thread = std::thread::current();
        Self {
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            detail,
            backtrace: Backtrace::force_capture().to_string(),
        }
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

/// Run `main` under the hook; on unwind log, offer `on_crash`, and
/// return the outcome for the host to terminate on.
///
/// `on_crash` runs after unwind on the caller (UI) thread only; an
/// emu-thread panic never reaches it (typed `WorkerUnavailable` path
/// covers that). `on_crash` itself is catch-guarded so a failing dialog
/// cannot void the preserved log.
pub fn run_guarded<R>(
    main: impl FnOnce() -> R,
    on_crash: impl FnOnce(&CrashReport),
) -> GuardOutcome<R> {
    install();
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => {
            let report = CrashReport::from_payload(payload.as_ref());
            log::error!("{report}");
            let _ = panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_crash(&report)));
            GuardOutcome::Crashed(report)
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
    fn guard_returns_completed_value() {
        let outcome = run_guarded(|| 42, |_| panic!("on_crash must not run"));
        assert!(matches!(outcome, GuardOutcome::Completed(42)));
    }

    #[test]
    fn guard_returns_crashed_in_process() {
        let outcome: GuardOutcome<()> = run_guarded(|| panic!("boom"), |_| {});
        match outcome {
            GuardOutcome::Crashed(report) => assert_eq!(report.message(), "boom"),
            GuardOutcome::Completed(()) => panic!("crash must not complete"),
        }
    }

    #[test]
    fn hook_record_shares_capture_shape() {
        let record = PanicRecord::capture("explicit panic".to_owned());
        assert!(!record.thread.is_empty());
        assert_eq!(record.detail, "explicit panic");
        assert!(!record.backtrace.is_empty());
    }
}
