//! Outermost crash containment: hook install plus guarded entry.
//! The hook preserves only the log; dialogs and termination are the host's job.

use std::{
    any::Any,
    backtrace::Backtrace,
    panic::{self, PanicHookInfo},
    sync::Once,
};

/// Install the process-wide panic hook (idempotent, chains previous).
/// Best-effort log for pre-guard windows; `run_guarded` is authoritative.
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

/// Crash exit code for hosts (Rust panic convention: 101).
pub const CRASH_EXIT_CODE: i32 = 101;

/// Guard result: normal completion or an unwound crash. `#[must_use]`
/// keeps a crash from silently passing as a clean exit.
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

/// Thread, detail, and backtrace captured once for hook and report.
#[derive(Debug)]
struct PanicRecord {
    thread: String,
    detail: String,
    backtrace: String,
}

impl PanicRecord {
    // Release keeps symtab only: names resolve, file:line does not.
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
    } else if let Some(error) = payload.downcast_ref::<Box<dyn std::error::Error + Send>>() {
        error.to_string()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Run `main` under the hook and exit with the crash code on unwind.
/// Hosts that map [`GuardOutcome`] themselves keep using `run_guarded`.
pub fn run_guarded_and_exit<R>(main: impl FnOnce() -> R) -> ! {
    match run_guarded(main) {
        // `run_guarded` already logged any crash report.
        GuardOutcome::Completed(_) => std::process::exit(0),
        GuardOutcome::Crashed(_) => std::process::exit(CRASH_EXIT_CODE),
    }
}

/// Run `main` under the hook; on unwind log the report and return it.
/// The outcome tells the host how to terminate; nothing is reused after a catch.
pub fn run_guarded<R>(main: impl FnOnce() -> R) -> GuardOutcome<R> {
    install();
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => {
            let report = CrashReport::from_payload(payload.as_ref());
            log::error!("{report}");
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
        let boxed: Box<dyn std::error::Error + Send> = Box::new(std::io::Error::other("io boom"));
        assert_eq!(payload_message(&boxed), "io boom");
        assert_eq!(payload_message(&42_u32), "non-string panic payload");
    }

    #[test]
    fn report_display_names_thread_and_message() {
        let report = CrashReport::from_payload(&"boom");
        let text = report.to_string();
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains("crashed on thread"), "{text}");
        assert!(text.contains("backtrace:"), "{text}");
        assert_eq!(report.message(), "boom");
    }

    #[test]
    fn guard_returns_completed_value() {
        let outcome = run_guarded(|| 42);
        assert!(matches!(outcome, GuardOutcome::Completed(42)));
    }

    #[test]
    fn guard_returns_crashed_in_process() {
        let outcome: GuardOutcome<()> = run_guarded(|| panic!("boom"));
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
