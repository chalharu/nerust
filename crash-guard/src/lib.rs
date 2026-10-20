//! Outermost crash containment: hook install plus guarded entry. Hook preserves only the record;
//! dialogs and termination are the host's job. Configuration travels by parameter, never globals.

use std::{
    any::Any,
    panic::{self, PanicHookInfo},
};

/// Record sink for the panic hook. Capturing sinks move in by value; no global slot.
pub type RecordSink = std::sync::Arc<dyn Fn(&PanicRecord) + Send + Sync + 'static>;

/// Default sink: stderr log line for the crash record.
pub fn default_sink(record: &PanicRecord) {
    eprintln!(
        "panic on thread '{}': {}\nbacktrace:\n{}",
        record.thread, record.detail, record.backtrace
    );
}

/// Backtrace source: hosts and tests inject; capturing sources move in by value.
pub type BacktraceSource = std::sync::Arc<dyn Fn() -> String + Send + Sync + 'static>;

/// Real backtrace capture for production installs.
pub fn capture_backtrace() -> String {
    std::backtrace::Backtrace::capture().to_string()
}

/// Install generation: a guard restores only when no later install is live.
static INSTALL_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// RAII panic-hook install: chains on construction, generation-guarded restore on drop.
/// Out-of-order drops keep the newer hook; `mem::forget` the guard for a permanent install.
pub struct HookGuard {
    chained: std::sync::Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>,
    epoch: u64,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // Generation-counted restore: only the newest live install owns
        // the hook slot. An older guard dropped late keeps the newer hook.
        if INSTALL_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != self.epoch {
            eprintln!("panic hook guard dropped out of order; keeping the newer hook");
            return;
        }
        let chained = std::sync::Arc::clone(&self.chained);
        panic::set_hook(Box::new(move |info| chained(info)));
    }
}

/// Install the hook with an explicit sink, chaining the previous hook.
/// Returns a guard: dropping restores the previous hook.
pub fn install_with(sink: impl Fn(&PanicRecord) + Send + Sync + 'static) -> HookGuard {
    install_with_source(sink, capture_backtrace)
}

/// Install the hook with an explicit sink and backtrace source.
/// Returns a guard: dropping restores the previous hook.
pub fn install_with_source(
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> String + Send + Sync + 'static,
) -> HookGuard {
    install_erased(std::sync::Arc::new(sink), std::sync::Arc::new(source))
}

/// Erased entry point for pre-boxed sinks and sources.
fn install_erased(sink: RecordSink, source: BacktraceSource) -> HookGuard {
    // Epoch is the counter value *after* our install; a later install
    // moves the counter past it, marking this guard stale.
    let epoch = INSTALL_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let previous = panic::take_hook();
    let chained: std::sync::Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static> =
        std::sync::Arc::from(previous);
    let for_hook = std::sync::Arc::clone(&chained);
    panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        let record = PanicRecord::capture_with(info.to_string(), source());
        sink(&record);
        for_hook(info);
    }));
    HookGuard { chained, epoch }
}

/// Install the default hook for the process lifetime. The host owns the guard;
/// dropping it restores the previous hook, so hosts keep it in a `static`.
pub fn install_default() -> HookGuard {
    install_with(default_sink)
}

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

    fn from_payload(payload: &(dyn Any + Send), backtrace: String) -> Self {
        Self {
            record: PanicRecord::capture_with(payload_message(payload), backtrace),
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
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> String + Send + Sync + 'static,
) -> GuardOutcome<R> {
    let source: BacktraceSource = std::sync::Arc::new(source);
    let for_hook = std::sync::Arc::clone(&source);
    let _guard = install_with_source(sink, move || for_hook());
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => {
            GuardOutcome::Crashed(CrashReport::from_payload(payload.as_ref(), source()))
        }
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
        let report = CrashReport::from_payload(&"boom", stub_source());
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
    fn capturing_sink_moves_in_without_globals() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::clone(&seen);
        // Capturing closure: no global slot, no fn-pointer gymnastics.
        let outcome: GuardOutcome<()> = run_guarded_with(
            || panic!("captured boom"),
            move |record: &PanicRecord| {
                probe.lock().unwrap().push(record.detail.clone());
            },
            || "stub backtrace".to_string(),
        );
        assert!(matches!(outcome, GuardOutcome::Crashed(_)));
        // Hook-path detail is the full hook info, not just the payload.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains("captured boom"), "{}", seen[0]);
    }

    #[test]
    fn nested_installs_restore_in_epoch_order() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        use std::sync::{Arc, Mutex};
        let outer_seen = Arc::new(Mutex::new(0usize));
        let inner_seen = Arc::new(Mutex::new(0usize));
        let outer_probe = Arc::clone(&outer_seen);
        let inner_probe = Arc::clone(&inner_seen);
        let outer = install_with_source(
            move |_| {
                *outer_probe.lock().unwrap() += 1;
            },
            stub_source,
        );
        let inner = install_with_source(
            move |_| {
                *inner_probe.lock().unwrap() += 1;
            },
            stub_source,
        );
        // Out-of-order drop: the older guard keeps the newer hook live.
        drop(outer);
        let result = panic::catch_unwind(|| panic!("nested probe"));
        assert!(result.is_err());
        assert_eq!(*inner_seen.lock().unwrap(), 1, "newer hook stays live");
        assert_eq!(*outer_seen.lock().unwrap(), 1, "chain still delegates");
        // Dropping the newest guard restores what it chained (the outer hook).
        drop(inner);
        let result = panic::catch_unwind(|| panic!("after inner drop"));
        assert!(result.is_err());
        assert_eq!(*inner_seen.lock().unwrap(), 1, "inner hook gone");
        assert_eq!(*outer_seen.lock().unwrap(), 2, "outer hook restored");
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
    fn report_covers_owned_and_unknown_payloads() {
        let owned = CrashReport::from_payload(&"owned".to_owned(), stub_source());
        assert_eq!(owned.message(), "owned");
        assert!(owned.to_string().contains("stub backtrace"));
        let unknown = CrashReport::from_payload(&42_u32, stub_source());
        assert_eq!(unknown.message(), "non-string panic payload");
    }

    #[test]
    fn record_getters_expose_thread_and_backtrace() {
        let report = CrashReport::from_payload(&"boom", stub_source());
        let text = format!("{report:?}");
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn guard_outcome_is_debug_and_must_use() {
        let completed: GuardOutcome<u32> = GuardOutcome::Completed(1);
        assert!(format!("{completed:?}").contains("Completed"));
        let crashed: GuardOutcome<u32> =
            GuardOutcome::Crashed(CrashReport::from_payload(&"x", stub_source()));
        assert!(format!("{crashed:?}").contains("Crashed"));
    }
}
