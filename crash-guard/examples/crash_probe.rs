//! Subprocess hook probe. Production code never references this.

fn main() {
    // Leaf design: the host injects the sink; the guard never touches `log`.
    nerust_crash_guard::set_record_sink(|record| {
        eprintln!(
            "panic on thread '{}': {}\nbacktrace:\n{}",
            record.thread(),
            record.detail(),
            record.backtrace()
        );
    });
    // The dialog runs in the Crashed arm, mirroring real frontends.
    // Exhaustive: a future outcome must not silently pass through.
    match nerust_crash_guard::run_guarded(|| panic!("probe crash")) {
        nerust_crash_guard::GuardOutcome::Completed(()) => {}
        nerust_crash_guard::GuardOutcome::Crashed(report) => {
            eprintln!("probe dialog would show: {}", report.message());
            std::process::exit(nerust_crash_guard::CRASH_EXIT_CODE);
        }
    }
}
