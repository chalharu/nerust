//! Subprocess hook probe. Production code never references this.

/// Minimal stderr logger: the leaf emits via the `log` facade only,
// so the probe must provide a backend (real frontends use SimpleLogger).
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn main() {
    log::set_logger(&LOGGER).expect("probe logger installs once");
    log::set_max_level(log::LevelFilter::Error);
    // The dialog runs in the Crashed arm, mirroring real frontends.
    if let nerust_crash_guard::GuardOutcome::Crashed(report) =
        nerust_crash_guard::run_guarded(|| panic!("probe crash"))
    {
        eprintln!("probe dialog would show: {}", report.message());
        std::process::exit(nerust_crash_guard::CRASH_EXIT_CODE);
    }
}
