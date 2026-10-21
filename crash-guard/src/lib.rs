//! Outermost crash containment: hook diagnostics plus guarded entry. Hook records describe
//! the panic site; caught reports describe only the caught payload, without claiming correlation.
//! Dialogs and termination are the host's job. Configuration travels by parameter, never globals.

use std::{
    any::Any,
    panic::{self, PanicHookInfo},
    sync::{Arc, Mutex},
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

type Hook = Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

struct Registration {
    sink: RecordSink,
    source: BacktraceSource,
    thread: Option<std::thread::ThreadId>,
}

struct Hooks {
    previous: Option<Hook>,
    registrations: Vec<Arc<Registration>>,
}

// Never hold HOOKS across a std hook write: std invokes hooks under its own
// read lock, and the dispatcher needs HOOKS to take its snapshot.
// INSTALL_LOCK serializes our take_hook/set_hook transactions only.
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static HOOKS: Mutex<Hooks> = Mutex::new(Hooks {
    previous: None,
    registrations: Vec::new(),
});

/// Owns one process-wide registration, not a snapshot of another guard's hook.
/// Drops may occur in any order or thread; only live registrations are called,
/// newest first, followed by the original process hook. In-flight snapshots may
/// finish after a guard drops. Forgetting the guard makes its registration permanent.
/// The newest source captures one record shared by all sinks. Coordinate external
/// `set_hook`/`take_hook` users: std offers no atomic compare-and-swap for hooks.
#[must_use = "keep the guard alive while its panic handler is needed"]
pub struct HookGuard {
    registration: Arc<Registration>,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // std forbids set_hook while unwinding. Leave an empty forwarding
        // dispatcher in that case; the next non-panicking guard drop restores it.
        // In particular, a guard dropped by a sink must not wait for another
        // install that is itself waiting for this panic hook to return.
        let can_restore = !std::thread::panicking();
        let _install = can_restore.then(|| {
            INSTALL_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        });
        let previous = {
            let mut hooks = HOOKS.lock().unwrap_or_else(|poison| poison.into_inner());
            hooks
                .registrations
                .retain(|entry| !Arc::ptr_eq(entry, &self.registration));
            if hooks.registrations.is_empty() && can_restore {
                hooks.previous.take()
            } else {
                None
            }
        };
        let replaced = if let Some(previous) = previous {
            // set_hook drops the replaced closure. Its captures may install a
            // new guard in Drop, so first take ownership without destroying it.
            // Neither std hook writes nor closure destruction may hold HOOKS.
            let replaced = panic::take_hook();
            panic::set_hook(Box::new(move |info| previous(info)));
            Some(replaced)
        } else {
            None
        };
        drop(_install);
        // In particular, a wrapping external hook can own a reentrant or
        // panicking destructor. The registry is already restored and unlocked.
        drop(replaced);
    }
}

/// Install the hook with an explicit sink, chaining the previous hook.
/// Returns a guard; the last non-panicking drop restores the previous hook.
pub fn install_with(sink: impl Fn(&PanicRecord) + Send + Sync + 'static) -> HookGuard {
    install_with_source(sink, capture_backtrace)
}

/// Install the hook with an explicit sink and backtrace source.
/// Returns a guard; the last non-panicking drop restores the previous hook.
pub fn install_with_source(
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> String + Send + Sync + 'static,
) -> HookGuard {
    install_erased(std::sync::Arc::new(sink), std::sync::Arc::new(source), None)
}

/// Erased entry point for pre-boxed sinks and sources.
fn install_erased(
    sink: RecordSink,
    source: BacktraceSource,
    thread: Option<std::thread::ThreadId>,
) -> HookGuard {
    // Check before locking: set_hook would panic under the lock and its hook
    // would otherwise deadlock trying to snapshot the same registry.
    assert!(
        !std::thread::panicking(),
        "cannot install a hook while panicking"
    );
    let registration = Arc::new(Registration {
        sink,
        source,
        thread,
    });
    let _install = INSTALL_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let needs_install = HOOKS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .previous
        .is_none();
    if needs_install {
        let previous: Hook = Arc::from(panic::take_hook());
        {
            let mut hooks = HOOKS.lock().unwrap_or_else(|poison| poison.into_inner());
            hooks.previous = Some(Arc::clone(&previous));
            hooks.registrations.push(Arc::clone(&registration));
        }
        panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current().id();
            let registrations = {
                let hooks = HOOKS.lock().unwrap_or_else(|poison| poison.into_inner());
                hooks
                    .registrations
                    .iter()
                    .filter(|entry| entry.thread.is_none_or(|owner| owner == thread))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            // Capture at the panic site once, using the newest live source.
            if let Some(newest) = registrations.last() {
                let record = PanicRecord::capture_with(info.to_string(), (newest.source)());
                for registration in registrations.iter().rev() {
                    (registration.sink)(&record);
                }
            }
            previous(info);
        }));
    } else {
        HOOKS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .registrations
            .push(Arc::clone(&registration));
    }
    HookGuard { registration }
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

/// Caught payload data for dialogs: getters only, log shape via `Display`.
/// Hook diagnostics are logged separately: stable std cannot establish their
/// identity with an arbitrary caught payload, especially after `resume_unwind`.
#[derive(Debug, Clone)]
pub struct CrashReport {
    thread: String,
    message: String,
}

const UNAVAILABLE_BACKTRACE: &str =
    "panic hook record unavailable for caught payload (correlation cannot be established)";

impl CrashReport {
    /// What panicked (payload text or fallback).
    pub fn message(&self) -> &str {
        &self.message
    }

    fn from_payload(payload: &(dyn Any + Send)) -> Self {
        Self {
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned(),
            message: payload_message(payload),
        }
    }
}

impl std::fmt::Display for CrashReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "crashed on thread '{}': {}\nbacktrace:\n{}",
            self.thread, self.message, UNAVAILABLE_BACKTRACE
        )
    }
}

/// Thread, hook detail, and panic-site backtrace captured once per hook invocation.
/// This diagnostic is not evidence about the payload eventually caught by a guard.
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

    /// Full panic hook detail, including the panic location when available.
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
/// Report construction is silent; caught state is never resumed or reused.
/// The hook logs each invocation with one panic-site capture shared by its sinks.
/// The report uses only the caught payload: hooks may belong to swallowed panics,
/// and `resume_unwind` skips hooks. Addresses, types, and equal text cannot prove
/// correlation (storage can move or be reused). Reports therefore always state
/// that the hook record is unavailable for the caught payload. No catch-site
/// trace is substituted and neither sink nor source is called after the catch.
pub fn run_guarded_with<R>(
    main: impl FnOnce() -> R,
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> String + Send + Sync + 'static,
) -> GuardOutcome<R> {
    let thread = std::thread::current().id();
    let _guard = install_erased(Arc::new(sink), Arc::new(source), Some(thread));
    match panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(value) => GuardOutcome::Completed(value),
        Err(payload) => GuardOutcome::Crashed(CrashReport::from_payload(payload.as_ref())),
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
        let report = CrashReport::from_payload(&"boom");
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
    fn out_of_order_drops_do_not_resurrect_dead_registrations() {
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
        assert_eq!(*outer_seen.lock().unwrap(), 0, "dropped hook is gone");
        // Dropping the newest guard restores the original, not the dead outer.
        drop(inner);
        let result = panic::catch_unwind(|| panic!("after inner drop"));
        assert!(result.is_err());
        assert_eq!(*inner_seen.lock().unwrap(), 1, "inner hook gone");
        assert_eq!(*outer_seen.lock().unwrap(), 0, "outer hook stays removed");
    }

    #[test]
    fn lifo_drops_restore_outer_then_original() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&calls);
        let outer = install_with_source(
            move |_| {
                probe.fetch_add(1, Ordering::SeqCst);
            },
            stub_source,
        );
        let inner = install_with_source(|_| {}, stub_source);
        drop(inner);
        let _ = panic::catch_unwind(|| panic!("outer only"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(outer);
        let _ = panic::catch_unwind(|| panic!("original only"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(HOOKS.lock().unwrap().previous.is_none());
    }

    #[test]
    fn concurrent_installs_and_drops_keep_all_live_owners() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(std::sync::Barrier::new(9));
        let done = Arc::new(std::sync::Barrier::new(9));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (calls, ready, done) =
                    (Arc::clone(&calls), Arc::clone(&ready), Arc::clone(&done));
                scope.spawn(move || {
                    let _guard = install_with_source(
                        move |_| {
                            calls.fetch_add(1, Ordering::SeqCst);
                        },
                        stub_source,
                    );
                    ready.wait();
                    done.wait();
                });
            }
            ready.wait();
            let _ = panic::catch_unwind(|| panic!("all live"));
            done.wait();
        });
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        let _ = panic::catch_unwind(|| panic!("all dropped"));
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        assert!(HOOKS.lock().unwrap().previous.is_none());
    }

    #[test]
    fn guarded_report_separates_hook_diagnostic_and_captures_once() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&calls);
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let outcome: GuardOutcome<()> = run_guarded_with(
            || panic!("origin"),
            move |record| {
                *sink.lock().unwrap() = Some(record.clone());
            },
            move || {
                probe.fetch_add(1, Ordering::SeqCst);
                "panic-site trace".to_owned()
            },
        );
        let GuardOutcome::Crashed(report) = outcome else {
            panic!("expected crash")
        };
        let seen = seen.lock().unwrap();
        let logged = seen.as_ref().unwrap();
        assert!(logged.detail().contains("origin"));
        assert!(logged.detail().contains("panicked at"));
        assert_eq!(logged.backtrace(), "panic-site trace");
        assert_eq!(report.message(), "origin");
        let text = report.to_string();
        assert!(text.contains(UNAVAILABLE_BACKTRACE), "{text}");
        assert!(!text.contains(logged.backtrace()), "{text}");
        assert!(!text.contains("panicked at"), "{text}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn nested_guarded_calls_share_one_capture_per_panic() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&calls);
        let outer_calls = Arc::new(AtomicUsize::new(0));
        let outer_source = Arc::clone(&outer_calls);
        let outer_seen = Arc::new(Mutex::new(Vec::new()));
        let outer_sink = Arc::clone(&outer_seen);
        let inner_seen = Arc::new(Mutex::new(Vec::new()));
        let inner_sink = Arc::clone(&inner_seen);
        let _process = install_with_source(|_| {}, || panic!("newest source must win"));
        let outer: GuardOutcome<()> = run_guarded_with(
            || {
                let inner: GuardOutcome<()> = run_guarded_with(
                    || panic!("inner"),
                    move |record| inner_sink.lock().unwrap().push(record.clone()),
                    move || {
                        probe.fetch_add(1, Ordering::SeqCst);
                        "inner trace".to_owned()
                    },
                );
                let GuardOutcome::Crashed(report) = inner else {
                    panic!("expected inner crash")
                };
                assert_eq!(report.message(), "inner");
                let text = report.to_string();
                assert!(text.contains(UNAVAILABLE_BACKTRACE), "{text}");
                assert!(!text.contains("inner trace"), "{text}");
                panic!("outer");
            },
            move |record| outer_sink.lock().unwrap().push(record.clone()),
            move || {
                outer_source.fetch_add(1, Ordering::SeqCst);
                "outer trace".to_owned()
            },
        );
        let GuardOutcome::Crashed(report) = outer else {
            panic!("expected crash")
        };
        assert_eq!(report.message(), "outer");
        let text = report.to_string();
        assert!(text.contains(UNAVAILABLE_BACKTRACE), "{text}");
        assert!(!text.contains("outer trace"), "{text}");
        assert!(!text.contains("inner trace"), "{text}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(outer_calls.load(Ordering::SeqCst), 1);
        let outer_seen = outer_seen.lock().unwrap();
        let inner_seen = inner_seen.lock().unwrap();
        assert_eq!(outer_seen.len(), 2);
        assert_eq!(inner_seen.len(), 1);
        assert_eq!(outer_seen[0].detail(), inner_seen[0].detail());
        assert_eq!(outer_seen[0].thread(), inner_seen[0].thread());
        assert_eq!(outer_seen[0].backtrace(), "inner trace");
        assert_eq!(inner_seen[0].backtrace(), "inner trace");
        assert_eq!(outer_seen[1].backtrace(), "outer trace");
    }

    #[test]
    fn concurrent_guarded_reports_do_not_take_other_threads_records() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|index| {
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        let outcome: GuardOutcome<()> = run_guarded_with(
                            || {
                                barrier.wait();
                                panic!("thread {index}");
                            },
                            |_| {},
                            move || format!("trace {index}"),
                        );
                        let GuardOutcome::Crashed(report) = outcome else {
                            panic!("expected crash")
                        };
                        assert_eq!(report.message(), format!("thread {index}"));
                        let text = report.to_string();
                        assert!(text.contains(UNAVAILABLE_BACKTRACE), "{text}");
                        assert!(!text.contains("trace "), "{text}");
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }
        });
    }

    #[test]
    fn drop_during_unwind_is_safe_and_next_scope_restores_original() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let _ = panic::catch_unwind(|| {
            let _guard = install_with_source(|_| {}, stub_source);
            panic!("unwind a guard");
        });
        let guard = install_with_source(|_| {}, stub_source);
        drop(guard);
        assert!(HOOKS.lock().unwrap().previous.is_none());
    }

    #[test]
    fn resumed_panic_without_hook_does_not_capture_at_catch_site() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let outcome: GuardOutcome<()> = run_guarded_with(
            || panic::resume_unwind(Box::new("resumed")),
            |_| panic!("resume must not call sink"),
            || panic!("must not capture after catch"),
        );
        let GuardOutcome::Crashed(report) = outcome else {
            panic!("expected crash")
        };
        assert_eq!(report.message(), "resumed");
        assert!(report.to_string().contains(UNAVAILABLE_BACKTRACE));
    }

    #[test]
    fn replaced_payload_in_same_allocation_never_reuses_old_diagnostic() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let captures = Arc::new(AtomicUsize::new(0));
        let source = Arc::clone(&captures);
        let outcome: GuardOutcome<()> = run_guarded_with(
            || {
                let mut payload =
                    panic::catch_unwind(|| panic::panic_any("obsolete-payload-A".to_owned()))
                        .unwrap_err()
                        .downcast::<String>()
                        .unwrap();
                let address = std::ptr::from_ref(payload.as_ref());
                // Drop the old value and replace it in the same live allocation.
                // No unsafe code or allocator behavior is needed to reuse storage.
                *payload = "replacement".to_owned();
                assert_eq!(address, std::ptr::from_ref(payload.as_ref()));
                panic::resume_unwind(payload);
            },
            move |record| sink.lock().unwrap().push(record.clone()),
            move || {
                source.fetch_add(1, Ordering::SeqCst);
                "old payload trace".to_owned()
            },
        );
        let GuardOutcome::Crashed(report) = outcome else {
            panic!("expected replaced payload crash")
        };
        assert_eq!(report.message(), "replacement");
        let text = report.to_string();
        assert!(text.contains(UNAVAILABLE_BACKTRACE), "{text}");
        assert!(!text.contains("obsolete-payload-A"), "{text}");
        assert!(!text.contains("old payload trace"), "{text}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].detail().contains("obsolete-payload-A"));
        assert_eq!(seen[0].backtrace(), "old payload trace");
        assert_eq!(captures.load(Ordering::SeqCst), 1);
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
        let owned = CrashReport::from_payload(&"owned".to_owned());
        assert_eq!(owned.message(), "owned");
        assert!(owned.to_string().contains(UNAVAILABLE_BACKTRACE));
        let unknown = CrashReport::from_payload(&42_u32);
        assert_eq!(unknown.message(), "non-string panic payload");
        assert!(unknown.to_string().contains(UNAVAILABLE_BACKTRACE));
    }

    #[test]
    fn record_getters_expose_thread_and_backtrace() {
        let record = PanicRecord::capture_with("boom".to_owned(), stub_source());
        assert_eq!(
            record.thread(),
            std::thread::current().name().unwrap_or("unnamed")
        );
        assert_eq!(record.detail(), "boom");
        assert_eq!(record.backtrace(), "stub backtrace");
        let text = format!("{record:?}");
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn guard_outcome_is_debug_and_must_use() {
        let completed: GuardOutcome<u32> = GuardOutcome::Completed(1);
        assert!(format!("{completed:?}").contains("Completed"));
        let crashed: GuardOutcome<u32> = GuardOutcome::Crashed(CrashReport::from_payload(&"x"));
        assert!(format!("{crashed:?}").contains("Crashed"));
    }
}
