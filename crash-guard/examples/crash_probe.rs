//! Probe child for the hook test: installs the guard and panics.
//! Never referenced by production code; located by the test via
//! `current_exe()/examples/crash_probe`.

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
    match nerust_crash_guard::run_guarded(
        || panic!("probe crash"),
        |report| {
            eprintln!("probe dialog would show: {}", report.message());
        },
    ) {
        // A crash must never look like a clean exit; hosts own this.
        nerust_crash_guard::GuardOutcome::Completed(()) => {}
        nerust_crash_guard::GuardOutcome::Crashed(_) => {
            std::process::exit(nerust_crash_guard::CRASH_EXIT_CODE)
        }
    }
}
