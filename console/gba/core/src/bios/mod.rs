pub mod decompress;
pub mod hle_operation;

mod compute;
mod system;

#[cfg(test)]
mod tests;

use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// Fresh bus for tests that only need default state.
#[cfg(test)]
pub(super) fn test_bus() -> GbaMemoryBus {
    GbaMemoryBus::new()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwiResult {
    Return(u32),
    Branch(u32),
    Unsupported,
}

/// HLE BIOS dispatcher.
pub fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8) -> SwiResult {
    let result = match swi {
        0x00..=0x05 | 0x20..=0x27 | 0x19..=0x1F | 0x28..=0x2A => system::dispatch(regs, bus, swi),
        0x06..=0x0F => compute::dispatch(regs, bus, swi),
        0x10..=0x18 => decompress::dispatch(regs, bus, swi),
        _ => SwiResult::Unsupported,
    };
    // GBATEK BIOS guard: leaving the BIOS region latches the last fetched
    // opcode for protected reads (jsmolka bios t002).
    if !matches!(result, SwiResult::Unsupported) {
        bus.set_bios_prefetch(0xE3A02004);
    }
    result
}
