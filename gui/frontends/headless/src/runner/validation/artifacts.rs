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
