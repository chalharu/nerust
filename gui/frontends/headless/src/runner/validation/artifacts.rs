pub(super) mod memory;
pub(super) mod registers;
mod screen;
mod summary;

#[derive(Default)]
pub(super) struct ValidationArtifacts {
    screen: screen::ScreenArtifacts,
    memory: memory::MemoryArtifacts,
    registers: registers::RegisterArtifacts,
    failures: Vec<String>,
}
