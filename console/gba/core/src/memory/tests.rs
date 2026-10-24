//! Memory-bus regression tests, split by area.

use super::GbaMemoryBus;

mod cpu_audio;
mod io_dma;
mod prefetch;

/// Fresh bus for tests that only need default state.
pub(super) fn test_bus() -> GbaMemoryBus {
    GbaMemoryBus::new()
}
