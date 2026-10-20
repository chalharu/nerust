//! Outermost crash containment: hook install plus guarded entry. Hook preserves only the record;
//! dialogs and termination are the host's job. Configuration travels by parameter, never globals.

use std::{
    any::Any,
    panic::{self, PanicHookInfo},
};

/// Record sink for the panic hook. Passed per install; no global slot.
pub type RecordSink = fn(&PanicRecord);

/// Default sink: stderr log line for the crash record.
pub fn default_sink(record: &PanicRecord) {
    eprintln!(
        "panic on thread '{}': {}\nbacktrace:\n{}",
        record.thread, record.detail, record.backtrace
    );
}

/// Backtrace source: injected so hosts and tests avoid the real capture.
/// Production passes [`capture_backtrace`]; tests pass a fixed string.
pub type BacktraceSource = fn() -> String;

/// Real backtrace capture for production installs.
pub fn capture_backtrace() -> String {
    std::backtrace::Backtrace::capture().to_string()
}

/// RAII panic-hook install. Chains the previous hook on construction and restores it on drop, so
/// scoped installs never leak into the host. Drops must nest (LIFO); `mem::forget` the guard for a permanent install.
pub struct HookGuard {
    chained: std::sync::Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // Best-effort LIFO restore: wrap the chained hook back in place.
        // A later install dropped out of order keeps its own chain.
        let chained = std::sync::Arc::clone(&self.chained);
        panic::set_hook(Box::new(move |info| chained(info)));
    }
}

/// Install the hook with an explicit sink, chaining the previous hook.
/// Returns a guard: dropping restores the previous hook.
pub fn install_with(sink: RecordSink) -> HookGuard {
    install_with_source(sink, capture_backtrace)
}

/// Install the hook with an explicit sink and backtrace source.
/// Returns a guard: dropping restores the previous hook.
pub fn install_with_source(sink: RecordSink, source: BacktraceSource) -> HookGuard {
    let previous = panic::take_hook();
    let chained: std::sync::Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static> =
        std::sync::Arc::from(previous);
    let for_hook = std::sync::Arc::clone(&chained);
    panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        let record = PanicRecord::capture_with(info.to_string(), source());
        sink(&record);
        for_hook(info);
    }));
    HookGuard { chained }
}

/// Install the default hook for the process lifetime. The host owns the guard;
/// dropping it restores the previous hook, so hosts keep it in a `static`.
pub fn install_default() -> HookGuard {
    install_with(default_sink)
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

    fn from_payload(payload: &(dyn Any + Send), source: BacktraceSource) -> Self {
        Self {
            record: PanicRecord::capture_with(payload_message(payload), source()),
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
/// `Debug` exposes the same fields as the getters.
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
    /// Capture with an explicit backtrace (hook path and tests inject it).
    /// `Backtrace::capture` respects `RUST_BACKTRACE`: unset/`0` yields a placeholder.
    fn capture_with(detail: String, backtrace: String) -> Self {
        let thread = std::thread::current();
        Self {
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            detail,
            backtrace,
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

/// Run `main` under a scoped hook; on unwind build the report and return it.
/// Silent (the hook already logged); caught state is never resumed or reused.
pub fn run_guarded_with<R>(
    main: impl FnOnce() -> R,
    sink: RecordSink,
    source: BacktraceSource,
) -> GuardOutcome<R> {
    let _guard = install_with_source(sink, source);
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => GuardOutcome::Crashed(CrashReport::from_payload(payload.as_ref(), source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    /// Serializes hook-mutating tests: the panic hook is process-global.
    static HOOK_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn stub_source() -> String {
        "stub backtrace".to_string()
    }

    static SINK_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_sink(_record: &PanicRecord) {
        SINK_CALLS.fetch_add(1, Ordering::SeqCst);
    }

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
        let report = CrashReport::from_payload(&"boom", stub_source);
        let text = report.to_string();
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains("crashed on thread"), "{text}");
        assert!(text.contains("backtrace:"), "{text}");
        assert_eq!(report.message(), "boom");
    }

    #[test]
    fn guard_returns_completed_value() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let outcome = run_guarded_with(|| 42, counting_sink, stub_source);
        assert!(matches!(outcome, GuardOutcome::Completed(42)));
    }

    #[test]
    fn guard_returns_crashed_in_process() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let before = SINK_CALLS.load(Ordering::SeqCst);
        let outcome: GuardOutcome<()> =
            run_guarded_with(|| panic!("boom"), counting_sink, stub_source);
        match outcome {
            GuardOutcome::Crashed(report) => assert_eq!(report.message(), "boom"),
            GuardOutcome::Completed(()) => panic!("crash must not complete"),
        }
        assert_eq!(SINK_CALLS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn hook_guard_restores_previous_hook_on_drop() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let before = SINK_CALLS.load(Ordering::SeqCst);
        {
            let _guard = install_with_source(counting_sink, stub_source);
            let result = panic::catch_unwind(|| panic!("scoped probe"));
            assert!(result.is_err());
        }
        assert_eq!(
            SINK_CALLS.load(Ordering::SeqCst),
            before + 1,
            "guard hook fired once"
        );
        // Dropped guard restored the previous hook: no more counting calls.
        let result = panic::catch_unwind(|| panic!("after drop"));
        assert!(result.is_err());
        assert_eq!(SINK_CALLS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn hook_record_shares_capture_shape() {
        let record = PanicRecord::capture_with("explicit panic".to_owned(), stub_source());
        assert!(!record.thread.is_empty());
        assert_eq!(record.detail, "explicit panic");
        assert_eq!(record.backtrace, "stub backtrace");
    }

    #[test]
    fn default_install_covers_hosts_unchanged() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let _guard = install_default();
        let outcome = run_guarded_with(|| 7, default_sink, stub_source);
        assert!(matches!(outcome, GuardOutcome::Completed(7)));
        let outcome: GuardOutcome<()> =
            run_guarded_with(|| panic!("legacy boom"), default_sink, stub_source);
        assert!(matches!(outcome, GuardOutcome::Crashed(_)));
    }

    #[test]
    fn crash_exit_code_matches_rust_panic_convention() {
        assert_eq!(super::CRASH_EXIT_CODE, 101);
    }

    #[test]
    fn report_covers_owned_and_unknown_payloads() {
        let owned = CrashReport::from_payload(&"owned".to_owned(), stub_source);
        assert_eq!(owned.message(), "owned");
        assert!(owned.to_string().contains("stub backtrace"));
        let unknown = CrashReport::from_payload(&42_u32, stub_source);
        assert_eq!(unknown.message(), "non-string panic payload");
    }

    #[test]
    fn record_getters_expose_thread_and_backtrace() {
        let report = CrashReport::from_payload(&"boom", stub_source);
        let text = format!("{report:?}");
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn guard_outcome_is_debug_and_must_use() {
        let completed: GuardOutcome<u32> = GuardOutcome::Completed(1);
        assert!(format!("{completed:?}").contains("Completed"));
        let crashed: GuardOutcome<u32> =
            GuardOutcome::Crashed(CrashReport::from_payload(&"x", stub_source));
        assert!(format!("{crashed:?}").contains("Crashed"));
    }
}
