pub(super) mod log;
pub(super) mod memory;
pub(super) mod registers;
mod screen;
pub(super) mod serial;
mod summary;

#[derive(Default)]
pub(super) struct ValidationArtifacts {
    screen: screen::ScreenArtifacts,
    memory: memory::MemoryArtifacts,
    registers: registers::RegisterArtifacts,
    serial: serial::SerialArtifacts,
    log: log::LogArtifacts,
    failures: Vec<String>,
    /// Stale prompts stashed by record fns (e.g. log allow-list
    /// graduations); drained into the outcome at finish(). Never
    /// tolerated: a fix must stay loud.
    stale_pending: Vec<String>,
}

/// Mixer observation passed to `finish` by value: artifacts receive
/// data, never the runtime, so recording code provably cannot step,
/// reset, or drive input.
pub(in crate::runner::validation) struct AudioSnapshot {
    pub(in crate::runner::validation) sample_rate: u32,
    pub(in crate::runner::validation) samples: u64,
    pub(in crate::runner::validation) hash: u64,
}

/// Cumulative bytes produced on one channel since power-on (or the
/// last reset). Unknown channels fail loudly (read-time validation
/// against the drained set, like registers/spaces) — never silently
/// empty.
pub(in crate::runner::validation) fn peek_serial<'m>(
    serial: &'m std::collections::HashMap<String, Vec<u8>>,
    channel: &str,
) -> Result<&'m [u8], crate::error::RomTestError> {
    serial.get(channel).map(Vec::as_slice).ok_or_else(|| {
        crate::error::RomTestError::EmuThread(format!(
            "unknown output channel `{channel}` (drained {:?})",
            serial.keys().collect::<Vec<_>>()
        ))
    })
}
