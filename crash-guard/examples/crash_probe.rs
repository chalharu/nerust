//! Subprocess hook probe. Production code never references this.

fn main() {
    // Leaf design: the host injects the sink; the guard never touches `log`.
    fn probe_sink(record: &nerust_crash_guard::PanicRecord) {
        eprintln!(
            "panic on thread '{}': {}\nbacktrace:\n{}",
            record.thread(),
            record.detail(),
            record.backtrace()
        );
    }
    fn probe_backtrace() -> String {
        // Deterministic source: the probe asserts shape, not real frames.
        "probe backtrace".to_string()
    }
    // The dialog runs in the Crashed arm, mirroring real frontends.
    // Exhaustive: a future outcome must not silently pass through.
    match nerust_crash_guard::run_guarded_with(
        || panic!("probe crash"),
        probe_sink,
        probe_backtrace,
    ) {
        nerust_crash_guard::GuardOutcome::Completed(()) => {}
        nerust_crash_guard::GuardOutcome::Crashed(report) => {
            eprintln!("probe dialog would show: {}", report.message());
            std::process::exit(nerust_crash_guard::CRASH_EXIT_CODE);
        }
    }
}
