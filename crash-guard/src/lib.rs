//! Outermost crash containment: hook install plus guarded entry.
//! The hook preserves only the log; dialogs and termination are the host's job.

use std::{
    any::Any,
    backtrace::Backtrace,
    panic::{self, PanicHookInfo},
    sync::Once,
};

/// Record sink for the panic hook. Production logs; tests capture.
///
/// Process-global [`std::sync::OnceLock`] semantics: the slot can be set
/// at most once per process. `set_record_sink` reports whether the current
/// call won the slot; tests that need distinct sinks must run in separate
/// processes (see `tests/hook_probe.rs`) or be serialised within one
/// process so only the first `set` takes effect.
type RecordSink = fn(&PanicRecord);

fn default_sink(record: &PanicRecord) {
    log::error!(
        "panic on thread '{}': {}\nbacktrace:\n{}",
        record.thread,
        record.detail,
        record.backtrace
    );
}

static SINK: std::sync::OnceLock<RecordSink> = std::sync::OnceLock::new();

/// Override the record sink (tests, embedders). Returns whether this
/// call won: a `false` return means a previous test already claimed the
/// process-wide slot, so the caller must not assume its sink is live.
pub fn set_record_sink(sink: RecordSink) -> bool {
    SINK.set(sink).is_ok()
}

fn emit(record: &PanicRecord) {
    SINK.get().copied().unwrap_or(default_sink)(record);
}

/// Install the process-wide panic hook (idempotent, chains previous).
/// Best-effort log for pre-guard windows; `run_guarded` is authoritative.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
            let record = PanicRecord::capture(info.to_string());
            emit(&record);
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
#[derive(Debug, Clone)]
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
/// Public so embedders can implement `set_record_sink`.
#[derive(Debug, Clone)]
pub struct PanicRecord {
    thread: String,
    detail: String,
    backtrace: String,
}

impl PanicRecord {
    /// Failing thread name.
    pub fn thread(&self) -> &str {
        &self.thread
    }

    /// Panic payload text.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Captured backtrace.
    pub fn backtrace(&self) -> &str {
        &self.backtrace
    }
}

impl PanicRecord {
    // Release keeps symtab only: names resolve, file:line does not.
    fn capture(detail: String) -> Self {
        // Symbolication runs even for caught panics: acceptable, as a
        // panic already unwinds (no hot path captures here).
        // `Backtrace::capture` respects `RUST_BACKTRACE`: unset/`0` yields
        // a disabled (short placeholder) backtrace, `1`/full captures frames.
        let thread = std::thread::current();
        Self {
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            detail,
            backtrace: Backtrace::capture().to_string(),
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
    } else if let Some(error) = payload.downcast_ref::<Box<dyn std::error::Error + Send + Sync>>() {
        error.to_string()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Run `main` under the hook; on unwind build the report and return it.
///
/// The panic hook already logged the crash, so this path stays silent and
/// lets the host decide how to surface `CrashReport`. Nothing is reused
/// after a catch.
///
/// `AssertUnwindSafe` is documented rather than assumed: `main` typically
/// owns the whole frontend setup closure, and a panic discards that state
/// instead of resuming it, so unwind-safety of captured references does not
/// need to hold across the catch boundary.
pub fn run_guarded<R>(main: impl FnOnce() -> R) -> GuardOutcome<R> {
    install();
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => GuardOutcome::Crashed(CrashReport::from_payload(payload.as_ref())),
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
        let boxed_sync: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("io sync boom"));
        assert_eq!(payload_message(&boxed_sync), "io sync boom");
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
        // `Backtrace::capture` respects `RUST_BACKTRACE`: without it the
        // backtrace is a disabled placeholder, with `1` it holds frames.
        // Either way the field must render non-empty so reports stay shaped.
        assert!(!record.backtrace.is_empty());
    }
}
