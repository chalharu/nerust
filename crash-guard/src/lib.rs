//! Outermost crash containment: hook diagnostics plus guarded entry; hosts own dialogs and termination.
//! Hook records describe the panic site; caught reports describe only the payload.

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
/// Returns shared text so one capture fans out to every sink without copying.
pub type BacktraceSource = std::sync::Arc<dyn Fn() -> Arc<str> + Send + Sync + 'static>;

/// Real backtrace capture for production installs.
pub fn capture_backtrace() -> Arc<str> {
    Arc::from(std::backtrace::Backtrace::capture().to_string())
}

type Hook = Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

struct Registration {
    sink: RecordSink,
    source: BacktraceSource,
    thread: Option<std::thread::ThreadId>,
}

/// Lifecycle of the single process-wide hook adapter; the dispatcher never takes it,
/// so std hook writes here cannot deadlock against panic delivery. Reinstalls adopt the live hook.
struct Lifecycle {
    previous: Option<Hook>,
    installed: bool,
    retired: Vec<Hook>,
}

static LIFECYCLE: Mutex<Lifecycle> = Mutex::new(Lifecycle {
    previous: None,
    installed: false,
    retired: Vec::new(),
});

/// Serializes hook swaps (first install, last restore). Swap paths hold it across the std
/// swap while owning no other lock; lock order is `HOOK_SWAP` -> `LIFECYCLE` -> `REGISTRATIONS`.
static HOOK_SWAP: Mutex<()> = Mutex::new(());

/// Live registrations snapshotted by the dispatcher; the lock never nests inside user
/// code or std hook operations.
static REGISTRATIONS: Mutex<Vec<Arc<Registration>>> = Mutex::new(Vec::new());

/// Drop retired dispatchers outside every lock: their captures may install
/// guards reentrantly, and the registry is already consistent without them.
fn drain_retired() {
    let retired = std::mem::take(
        &mut LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retired,
    );
    drop(retired);
}

/// Owns one process-wide registration; drops may occur in any order or thread, and only
/// live registrations are called, newest first, followed by the original process hook.
#[must_use = "keep the guard alive while its panic handler is needed"]
pub struct HookGuard {
    registration: Arc<Registration>,
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // std forbids set_hook while unwinding: retain the registration but leave the
        // dispatcher; the next non-panicking guard drop restores it.
        let can_restore = !std::thread::panicking();
        drain_retired();
        if !can_restore {
            let mut registrations = REGISTRATIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registrations.retain(|entry| !Arc::ptr_eq(entry, &self.registration));
            return;
        }
        // The check, reserve, swap, and retire serialize under `HOOK_SWAP`, so a live
        // guard is never orphaned by a racing install.
        let _swap = HOOK_SWAP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = {
            let mut lifecycle = LIFECYCLE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut registrations = REGISTRATIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registrations.retain(|entry| !Arc::ptr_eq(entry, &self.registration));
            if lifecycle.installed && registrations.is_empty() {
                lifecycle.installed = false;
                lifecycle.previous.as_ref().map(Arc::clone)
            } else {
                None
            }
        };
        let Some(previous) = restore else { return };
        // No locks held: restore the process hook outside every adapter lock.
        let replaced = panic::take_hook();
        panic::set_hook(Box::new(move |info| previous(info)));
        LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retired
            .push(Arc::from(replaced));
    }
}

/// Install the hook with an explicit sink and backtrace source, chaining the previous
/// hook. The last non-panicking drop restores the previous hook.
pub fn install(
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> Arc<str> + Send + Sync + 'static,
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
    drain_retired();
    // Fast path: dispatcher already live. Publishing under `HOOK_SWAP` serializes against
    // a concurrent final drop, so a live guard is never orphaned.
    let _swap = HOOK_SWAP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    {
        let lifecycle = LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lifecycle.installed {
            REGISTRATIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Arc::clone(&registration));
            return HookGuard { registration };
        }
    }
    // Slow path: idle adapter. `HOOK_SWAP` stays held across reserve, swap, and publish,
    // so a racing last-drop restore cannot interleave.
    let taken = panic::take_hook();
    let previous = {
        let mut lifecycle = LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Adopt the live hook: an external `set_hook` while idle supersedes the stored
        // one, which is retired instead of the live hook.
        let live: Hook = Arc::from(taken);
        if let Some(stale) = lifecycle.previous.replace(Arc::clone(&live)) {
            lifecycle.retired.push(stale);
        }
        live
    };
    panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().id();
        let registrations = {
            let registrations = REGISTRATIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registrations
                .iter()
                .filter(|entry| entry.thread.is_none_or(|owner| owner == thread))
                .cloned()
                .collect::<Vec<_>>()
        };
        // Capture at the panic site once, using the newest live source.
        if let Some(newest) = registrations.last() {
            let record = PanicRecord::capture_with(Arc::from(info.to_string()), (newest.source)());
            for registration in registrations.iter().rev() {
                (registration.sink)(&record);
            }
        }
        previous(info);
    }));
    {
        let mut lifecycle = LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lifecycle.installed = true;
        REGISTRATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Arc::clone(&registration));
    }
    HookGuard { registration }
}

/// Install the default hook for the process lifetime. The host owns the guard;
/// dropping it restores the previous hook, so hosts keep it in a `static`.
pub fn install_default() -> HookGuard {
    install(default_sink, capture_backtrace)
}

/// Guard result: normal completion or an unwound crash. `#[must_use]`
/// keeps a crash from silently passing as a clean exit.
#[must_use]
#[derive(Debug)]
pub enum GuardOutcome<R> {
    Completed(R),
    Crashed(CrashReport),
}

/// Caught payload data for dialogs: getters only, log shape via `Display`. Hook diagnostics
/// are logged separately and never correlated with a caught payload.
#[derive(Debug, Clone)]
pub struct CrashReport {
    thread: Arc<str>,
    message: Arc<str>,
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
                .map_or(Arc::from("unnamed"), Arc::from),
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
#[derive(Debug, Clone)]
pub struct PanicRecord {
    thread: Arc<str>,
    detail: Arc<str>,
    backtrace: Arc<str>,
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
    fn capture_with(detail: Arc<str>, backtrace: Arc<str>) -> Self {
        let thread = std::thread::current();
        Self {
            thread: thread.name().map_or(Arc::from("unnamed"), Arc::from),
            detail,
            backtrace,
        }
    }
}

fn payload_message(payload: &(dyn Any + Send)) -> Arc<str> {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        Arc::from(*message)
    } else if let Some(message) = payload.downcast_ref::<String>() {
        Arc::from(message.as_str())
    } else if let Some(error) = payload.downcast_ref::<Box<dyn std::error::Error + Send>>() {
        Arc::from(error.to_string())
    } else if let Some(error) = payload.downcast_ref::<Box<dyn std::error::Error + Send + Sync>>() {
        Arc::from(error.to_string())
    } else {
        Arc::from("non-string panic payload")
    }
}

/// Run `main` under a scoped hook; on unwind build the report and return it.
/// Reports state that the hook record is unavailable for the caught payload.
pub fn run_guarded_with<R>(
    main: impl FnOnce() -> R,
    sink: impl Fn(&PanicRecord) + Send + Sync + 'static,
    source: impl Fn() -> Arc<str> + Send + Sync + 'static,
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

    fn stub_source() -> Arc<str> {
        Arc::from("stub backtrace")
    }

    static SINK_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// Process hook restored and nothing live or retired: reinstalls reuse the cached hook.
    fn adapter_is_idle() -> bool {
        drain_retired();
        let lifecycle = LIFECYCLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registrations = REGISTRATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !lifecycle.installed && registrations.is_empty() && lifecycle.retired.is_empty()
    }

    fn counting_sink(_record: &PanicRecord) {
        SINK_CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn payload_message_covers_string_forms() {
        assert_eq!(payload_message(&"static").as_ref(), "static");
        assert_eq!(payload_message(&"owned".to_owned()).as_ref(), "owned");
        let boxed: Box<dyn std::error::Error + Send> = Box::new(std::io::Error::other("io boom"));
        assert_eq!(payload_message(&boxed).as_ref(), "io boom");
        let boxed_sync: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("io sync boom"));
        assert_eq!(payload_message(&boxed_sync).as_ref(), "io sync boom");
        assert_eq!(
            payload_message(&42_u32).as_ref(),
            "non-string panic payload"
        );
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
            || Arc::from("stub backtrace"),
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
        let outer = install(
            move |_| {
                *outer_probe.lock().unwrap() += 1;
            },
            stub_source,
        );
        let inner = install(
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
        let outer = install(
            move |_| {
                probe.fetch_add(1, Ordering::SeqCst);
            },
            stub_source,
        );
        let inner = install(|_| {}, stub_source);
        drop(inner);
        let _ = panic::catch_unwind(|| panic!("outer only"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(outer);
        let _ = panic::catch_unwind(|| panic!("original only"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(adapter_is_idle());
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
                    let _guard = install(
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
        assert!(adapter_is_idle());
    }

    #[test]
    fn reinstall_race_never_orphans_or_doubles_live_deliveries() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let fixed = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&fixed);
        let keep_a = install(
            move |_| {
                probe.fetch_add(1, Ordering::SeqCst);
            },
            stub_source,
        );
        let fixed_b = Arc::new(AtomicUsize::new(0));
        let probe_b = Arc::clone(&fixed_b);
        let keep_b = install(
            move |_| {
                probe_b.fetch_add(1, Ordering::SeqCst);
            },
            stub_source,
        );
        // Churn final drops and reinstalls while the keeps stay live: the
        // adapter is never empty, so every panic reaches each keep exactly once.
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..50 {
                        let guard = install(|_| {}, stub_source);
                        drop(guard);
                    }
                });
            }
            for _ in 0..50 {
                let extra = install(|_| {}, stub_source);
                let _ = panic::catch_unwind(|| panic!("race probe"));
                drop(extra);
            }
        });
        assert_eq!(fixed.load(Ordering::SeqCst), 50);
        assert_eq!(fixed_b.load(Ordering::SeqCst), 50);
        drop(keep_a);
        drop(keep_b);
        // Churn never orphans a published guard: a final drop racing an
        // install serializes against the publish, so each live guard fires once.
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..50 {
                        let guard = install(|_| {}, stub_source);
                        drop(guard);
                    }
                });
            }
            for _ in 0..100 {
                let fired = Arc::new(AtomicUsize::new(0));
                let probe = Arc::clone(&fired);
                let guard = install(
                    move |_| {
                        probe.fetch_add(1, Ordering::SeqCst);
                    },
                    stub_source,
                );
                let _ = panic::catch_unwind(|| panic!("gap probe"));
                assert_eq!(
                    fired.load(Ordering::SeqCst),
                    1,
                    "live install survives churn"
                );
                drop(guard);
            }
        });
        assert!(adapter_is_idle());
    }

    /// A final drop racing a fresh publish must not orphan the live guard: every
    /// interleaving must still deliver exactly once.
    #[test]
    fn publish_racing_final_drop_never_orphans_live_guard() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        for _ in 0..200 {
            let doomed = install(|_| {}, stub_source);
            let gate = Arc::new(std::sync::Barrier::new(2));
            let fired = Arc::new(AtomicUsize::new(0));
            std::thread::scope(|scope| {
                let (gate, fired) = (Arc::clone(&gate), Arc::clone(&fired));
                let worker_gate = Arc::clone(&gate);
                scope.spawn(move || {
                    worker_gate.wait();
                    drop(doomed);
                });
                gate.wait();
                let probe = Arc::clone(&fired);
                let fresh = install(
                    move |_| {
                        probe.fetch_add(1, Ordering::SeqCst);
                    },
                    stub_source,
                );
                let _ = panic::catch_unwind(|| panic!("race probe"));
                assert_eq!(fired.load(Ordering::SeqCst), 1, "racing publish stays live");
                drop(fresh);
            });
            assert!(adapter_is_idle());
        }
    }

    /// A sink drop racing installer swaps on another thread always finishes.
    #[test]
    fn sink_drop_racing_install_completes() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let spare = Arc::new(Mutex::new(Some(install(|_| {}, stub_source))));
        let fired = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..100 {
                    let guard = install(|_| {}, stub_source);
                    drop(guard);
                }
            });
            let (spare, probe) = (Arc::clone(&spare), Arc::clone(&fired));
            let guard = install(
                move |_| {
                    probe.fetch_add(1, Ordering::SeqCst);
                    drop(spare.lock().unwrap().take());
                },
                stub_source,
            );
            let _ = panic::catch_unwind(|| panic!("sink drop probe"));
            assert_eq!(fired.load(Ordering::SeqCst), 1);
            drop(guard);
        });
        assert!(spare.lock().unwrap().is_none());
        assert!(adapter_is_idle());
    }

    /// Reinstall cycles never grow the forwarding depth: each live install observes
    /// exactly one delivery per panic.
    #[test]
    fn reinstall_cycles_keep_single_forwarding_layer() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        for _ in 0..3 {
            let fired = Arc::new(AtomicUsize::new(0));
            let probe = Arc::clone(&fired);
            let guard = install(
                move |_| {
                    probe.fetch_add(1, Ordering::SeqCst);
                },
                stub_source,
            );
            let _ = panic::catch_unwind(|| panic!("single layer probe"));
            assert_eq!(fired.load(Ordering::SeqCst), 1, "one delivery per panic");
            drop(guard);
            let _ = panic::catch_unwind(|| panic!("restored probe"));
            assert_eq!(fired.load(Ordering::SeqCst), 1, "dropped guard stays gone");
        }
        assert!(adapter_is_idle());
    }

    /// An external `set_hook` while idle survives the next install, and the final drop
    /// restores it (not the older stored hook).
    #[test]
    fn reinstall_adopts_external_hook_set_while_idle() {
        let _lock = HOOK_TEST_LOCK.lock().unwrap();
        let keep = install(|_| {}, stub_source);
        drop(keep);
        assert!(adapter_is_idle());
        // External set while idle: H1 supersedes the restored hook.
        let h1_calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&h1_calls);
        let idle_hook = panic::take_hook();
        panic::set_hook(Box::new(move |_| {
            probe.fetch_add(1, Ordering::SeqCst);
        }));
        let reinstall = install(|_| {}, stub_source);
        let _ = panic::catch_unwind(|| panic!("external must fire"));
        assert_eq!(h1_calls.load(Ordering::SeqCst), 1, "H1 survives reinstall");
        drop(reinstall);
        let _ = panic::catch_unwind(|| panic!("restored must fire"));
        assert_eq!(h1_calls.load(Ordering::SeqCst), 2, "final drop restores H1");
        // Hand the pre-test hook back; the adapter re-adopts it on next use.
        panic::set_hook(idle_hook);
        assert!(adapter_is_idle());
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
                Arc::from("panic-site trace")
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
        let _process = install(|_| {}, || panic!("newest source must win"));
        let outer: GuardOutcome<()> = run_guarded_with(
            || {
                let inner: GuardOutcome<()> = run_guarded_with(
                    || panic!("inner"),
                    move |record| inner_sink.lock().unwrap().push(record.clone()),
                    move || {
                        probe.fetch_add(1, Ordering::SeqCst);
                        Arc::from("inner trace")
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
                Arc::from("outer trace")
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
                            move || Arc::from(format!("trace {index}").as_str()),
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
            let _guard = install(|_| {}, stub_source);
            panic!("unwind a guard");
        });
        let guard = install(|_| {}, stub_source);
        drop(guard);
        assert!(adapter_is_idle());
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
                Arc::from("old payload trace")
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
            let _guard = install(counting_sink, stub_source);
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
        let record = PanicRecord::capture_with(Arc::from("explicit panic"), stub_source());
        assert!(!record.thread.is_empty());
        assert_eq!(record.detail.as_ref(), "explicit panic");
        assert_eq!(record.backtrace.as_ref(), "stub backtrace");
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
        let record = PanicRecord::capture_with(Arc::from("boom"), stub_source());
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
