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
    failures: Vec<String>,
}
