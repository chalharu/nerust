//! Subprocess probe: the hook + `run_guarded` touch process-global
//! state, so firing is verified out-of-process (exit code + stderr).

use std::{
    panic,
    process::{Command, Output, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use nerust_crash_guard::{GuardOutcome, install_with_source, run_guarded_with};

/// A deadlock in process-global hook handling must fail, not hang the suite.
fn output_with_timeout(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("probe spawn");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().expect("probe output"),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("reap probe");
                panic!(
                    "probe timed out or wait failed ({result:?}): {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

fn probe_binary() -> std::path::PathBuf {
    let mut dir = std::env::current_exe().expect("test executable path");
    dir.pop();
    if dir.file_name().is_some_and(|name| name == "deps") {
        dir.pop();
    }
    dir.join("examples").join("crash_probe")
}

#[test]
fn guard_exits_101_with_logged_report() {
    let binary = probe_binary();
    assert!(binary.is_file(), "probe not built: {}", binary.display());
    let output = output_with_timeout(Command::new(&binary).env("RUST_BACKTRACE", "0"));
    assert_eq!(output.status.code(), Some(101), "probe must exit(101)");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("probe crash"), "{stderr}");
    // The panic hook is the sole logger; `run_guarded` builds the report
    // silently for the host dialog (see the probe's "dialog would show" line).
    assert!(stderr.contains("panic on thread"), "{stderr}");
    assert_eq!(stderr.matches("panic on thread").count(), 1, "{stderr}");
    assert_eq!(stderr.matches("probe backtrace").count(), 1, "{stderr}");
    assert!(
        stderr.contains("probe dialog would show: probe crash"),
        "{stderr}"
    );
}

fn run_probe(scenario: &str) {
    let output = output_with_timeout(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "isolated_hook_probe", "--nocapture"])
            .env("NERUST_CRASH_GUARD_PROBE", scenario),
    );
    assert!(
        output.status.success(),
        "{scenario}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn replaced_hook_destructor_can_reinstall() {
    run_probe("reinstall");
}

#[test]
fn replaced_hook_destructor_can_unwind_and_recover() {
    run_probe("unwind");
}

#[test]
fn nested_concurrent_guards_keep_hook_diagnostics_separate() {
    run_probe("payloads");
}

#[test]
fn panic_snapshot_and_last_drop_do_not_invert_std_hook_lock() {
    run_probe("lock-order");
}

struct ReinstallOnDrop {
    calls: Arc<AtomicUsize>,
    unwind: bool,
}

impl Drop for ReinstallOnDrop {
    fn drop(&mut self) {
        let _guard = install_with_source(|_| {}, || "reentrant trace".to_owned());
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.unwind {
            panic!("external hook destructor");
        }
    }
}

fn replaced_hook_probe(unwind: bool) {
    let calls = Arc::new(AtomicUsize::new(0));
    let guard = install_with_source(|_| {}, || "original trace".to_owned());
    let previous = panic::take_hook();
    let capture = ReinstallOnDrop {
        calls: Arc::clone(&calls),
        unwind,
    };
    panic::set_hook(Box::new(move |info| {
        let _keep_capture_alive = &capture;
        previous(info);
    }));
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| drop(guard)));
    assert_eq!(result.is_err(), unwind);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // An unwind leaves a forwarding dispatcher; a later normal drop repairs it.
    let seen = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&seen);
    let guard = install_with_source(
        move |_| {
            sink.fetch_add(1, Ordering::SeqCst);
        },
        || "recovery trace".to_owned(),
    );
    let _ = panic::catch_unwind(|| panic!("live recovery guard"));
    drop(guard);
    let _ = panic::catch_unwind(|| panic!("restored original"));
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

fn assert_unrelated_payload<A: Send + 'static, B: Send + 'static>(
    first: A,
    second: B,
    expected: &str,
    drop_first: bool,
) {
    let captures = Arc::new(AtomicUsize::new(0));
    let source = Arc::clone(&captures);
    let outcome: GuardOutcome<()> = run_guarded_with(
        || {
            let first = panic::catch_unwind(panic::AssertUnwindSafe(|| panic::panic_any(first)))
                .unwrap_err();
            // Exercise both distinct live allocations and allocator reuse after
            // dropping A. Do not rely on any particular allocator reusing it.
            let _keep_first_alive = if drop_first {
                drop(first);
                None
            } else {
                Some(first)
            };
            panic::resume_unwind(Box::new(second));
        },
        |_| {},
        move || {
            source.fetch_add(1, Ordering::SeqCst);
            "unrelated A trace".to_owned()
        },
    );
    let GuardOutcome::Crashed(report) = outcome else {
        panic!("expected B crash")
    };
    assert_eq!(report.message(), expected);
    let text = report.to_string();
    assert!(
        text.contains("panic hook record unavailable for caught payload"),
        "{text}"
    );
    assert!(!text.contains("unrelated A trace"), "{text}");
    assert!(text.contains("correlation cannot be established"), "{text}");
    assert!(!text.contains("panicked at"), "{text}");
    assert_eq!(captures.load(Ordering::SeqCst), 1);
}

fn payload_probe() {
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..4 {
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || {
                let outer = run_guarded_with(
                    || {
                        barrier.wait();
                        for drop_first in [false, true] {
                            assert_unrelated_payload("A", "B", "B", drop_first);
                            assert_unrelated_payload("same", "same", "same", drop_first);
                            assert_unrelated_payload(
                                "same".to_owned(),
                                "same".to_owned(),
                                "same",
                                drop_first,
                            );
                            assert_unrelated_payload("same", "same".to_owned(), "same", drop_first);
                            assert_unrelated_payload(
                                42_u32,
                                42_u32,
                                "non-string panic payload",
                                drop_first,
                            );
                        }
                        let no_hook: GuardOutcome<()> = run_guarded_with(
                            || panic::resume_unwind(Box::new("no hook")),
                            |_| panic!("resume must not call sink"),
                            || panic!("resume must not call source"),
                        );
                        let GuardOutcome::Crashed(report) = no_hook else {
                            panic!("expected resumed crash")
                        };
                        assert_eq!(report.message(), "no hook");
                        assert!(report.to_string().contains("record unavailable"));
                        7
                    },
                    |_| {},
                    || panic!("inner source must win"),
                );
                assert!(matches!(outer, GuardOutcome::Completed(7)));
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
}

fn lock_order_probe() {
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..200 {
                let guard = install_with_source(|_| std::thread::yield_now(), String::new);
                std::thread::yield_now();
                drop(guard);
            }
        });
        scope.spawn(|| {
            for _ in 0..200 {
                let _ = panic::catch_unwind(|| panic!("concurrent panic"));
            }
        });
    });
}

#[test]
#[ignore = "invoked only by timeout-supervised parent tests"]
fn isolated_hook_probe() {
    let scenario = std::env::var("NERUST_CRASH_GUARD_PROBE").expect("parent probe scenario");
    panic::set_hook(Box::new(|_| {}));
    match scenario.as_str() {
        "reinstall" => replaced_hook_probe(false),
        "unwind" => replaced_hook_probe(true),
        "payloads" => payload_probe(),
        "lock-order" => lock_order_probe(),
        _ => panic!("unknown probe scenario: {scenario}"),
    }
}
