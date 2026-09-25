use crate::cartridge::Cartridge;
use crate::cpu::{GbaCpu, GbaCpuState};
use crate::memory::{GbaMemoryBus, GbaMemoryBusState};

pub struct GbaSystem {
    pub cpu: GbaCpu,
    pub bus: GbaMemoryBus,
    tick: u64,
    cpu_cycles_remaining: u32,
}

/// Phase 10 wire state: the T-cycle clock, the in-flight instruction
/// remainder, CPU and full bus state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GbaSystemState {
    tick: u64,
    cpu_cycles_remaining: u32,
    cpu: GbaCpuState,
    bus: GbaMemoryBusState,
}

impl GbaSystemState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        // The largest single charges are IRQ entries and HLE steps;
        // anything above a frame of T-cycles cannot be legitimate.
        if self.cpu_cycles_remaining > 280_896 {
            return Err(format!(
                "system: cpu_cycles_remaining out of range: {}",
                self.cpu_cycles_remaining
            ));
        }
        self.cpu.validate().map_err(|e| format!("system: {e}"))?;
        self.bus.validate().map_err(|e| format!("system: {e}"))?;
        // Absolute-time fields across devices share the T-cycle clock.
        if self.bus.current_tcycle > self.tick.saturating_add(1024) {
            return Err("system: bus clock ahead of system tick".to_string());
        }
        if self.tick > self.bus.current_tcycle.saturating_add(1024) {
            return Err("system: system tick ahead of bus clock".to_string());
        }
        if self.bus.timers.current_cycle > self.tick.saturating_add(1024) {
            return Err("system: timer clock ahead of system tick".to_string());
        }
        Ok(())
    }
}

impl GbaSystem {
    pub fn new() -> Self {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            tick: 0,
            cpu_cycles_remaining: 0,
        }
    }

    pub fn from_rom(rom: Vec<u8>) -> Option<Self> {
        let cart = Cartridge::new(rom)?;
        if !cart.header.logo_valid || !cart.header.fixed_valid || !cart.header.complement_valid {
            return None;
        }
        Some(Self::from_cartridge(cart))
    }

    /// Load third-party test ROMs that intentionally use a non-standard logo byte.
    /// Fixed-byte and complement validation remain mandatory.
    pub fn from_test_rom(rom: Vec<u8>) -> Option<Self> {
        let cart = Cartridge::new(rom)?;
        if !cart.header.fixed_valid || !cart.header.complement_valid {
            return None;
        }
        Some(Self::from_cartridge(cart))
    }

    fn from_cartridge(cart: Cartridge) -> Self {
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(cart);
        let mut cpu = GbaCpu::post_bios();
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            tick: 0,
            cpu_cycles_remaining: 0,
        }
    }

    pub fn bus(&self) -> &GbaMemoryBus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut GbaMemoryBus {
        &mut self.bus
    }

    pub fn cpu(&self) -> &GbaCpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut GbaCpu {
        &mut self.cpu
    }

    pub(crate) fn export_state(&self) -> Result<GbaSystemState, String> {
        Ok(GbaSystemState {
            tick: self.tick,
            cpu_cycles_remaining: self.cpu_cycles_remaining,
            cpu: self.cpu.export_state(),
            bus: self.bus.export_state()?,
        })
    }

    pub(crate) fn import_state(&mut self, state: GbaSystemState) -> Result<(), String> {
        state.validate()?;
        self.tick = state.tick;
        self.cpu_cycles_remaining = state.cpu_cycles_remaining;
        self.cpu.import_state(state.cpu)?;
        self.bus.import_state(state.bus)?;
        Ok(())
    }

    pub fn frame_buffer(&self) -> &[u32] {
        self.bus.frame_buffer()
    }

    pub fn run_frame(&mut self) -> &[u32] {
        while !self.step_tcycle() {}
        self.frame_buffer()
    }

    /// Advance up to `max` T-cycles with horizon batching, returning
    /// `(advanced, frame_complete)`. Cycles that need full per-cycle
    /// processing (CPU bus access, device events, pipeline deadlines) run
    /// through the untouched `step_tcycle` body; quiet spans advance
    /// clocks and free-running counters arithmetically. Bit-identical to
    /// the equivalent number of `step_tcycle` calls.
    pub fn step_batch(&mut self, max: u64) -> (u64, bool) {
        let mut advanced = 0u64;
        while advanced < max {
            let horizon = self.batch_horizon();
            if horizon == 0 {
                if self.step_tcycle() {
                    return (advanced + 1, true);
                }
                advanced += 1;
            } else {
                // Bound single jumps (hung states would otherwise advance
                // astronomically; the loop still never terminates there,
                // exactly like the per-cycle version).
                let take = (max - advanced).min(horizon).min(u64::from(u32::MAX));
                self.advance_idle(take, !self.bus.dma_active());
                advanced += take;
            }
        }
        (advanced, false)
    }

    /// Quiet prefix length before the next cycle needing full processing:
    /// CPU bus access (0 when the CPU acts this cycle), device events and
    /// IRQ pipeline deadlines.
    fn batch_horizon(&self) -> u64 {
        if !self.bus.dma_active() && !self.bus.is_halted() && self.cpu_cycles_remaining == 0 {
            return 0;
        }
        let mut horizon = self.bus.quiet_cycles();
        if !self.bus.is_halted() && !self.bus.dma_active() {
            horizon = horizon.min(u64::from(self.cpu_cycles_remaining));
        }
        horizon
    }

    /// Advance clocks and free-running counters by `n` cycles with no event
    /// processing. Valid only for `n <= batch_horizon()` at the same state.
    /// `fold_remaining` mirrors `step_tcycle`, which skips the remainder
    /// decrement while DMA owns the bus (unreachable here by the horizon,
    /// kept for exactness).
    fn advance_idle(&mut self, n: u64, fold_remaining: bool) {
        debug_assert!(n > 0);
        self.tick += n;
        if fold_remaining {
            self.cpu_cycles_remaining = self
                .cpu_cycles_remaining
                .saturating_sub(n.min(u64::from(u32::MAX)) as u32);
        }
        self.cpu
            .registers_mut()
            .tick_ldm_conflict_n(n.min(u64::from(u8::MAX)) as u8);
        self.bus.advance_idle(n);
    }

    /// Drain micro-ops within one tick: run ops while they cost nothing
    /// yet; stop at the first tick-consuming op (or retire). Returns the
    /// raw tick budget (possibly zero/negative; the caller floors once
    /// per instruction at retire, exactly like the legacy step), or None
    /// on an uncovered fill (queue empty there by construction).
    fn drain_micro(&mut self) -> Option<i64> {
        let mut acc = 0i64;
        loop {
            acc += self.cpu.step_op(&mut self.bus)?;
            if !self.cpu.micro_pending() {
                break;
            }
            if acc >= 1 {
                break;
            }
        }
        Some(acc)
    }

    /// CPUとバスを1 T-cycleだけ進行する。
    pub fn step_tcycle(&mut self) -> bool {
        if self.bus.dma_active() {
            // HW behavior: the CPU is stalled for the whole burst;
            // only the bus advances, the in-flight op resumes afterwards.
        } else {
            if !self.bus.is_halted() && self.cpu_cycles_remaining == 0 {
                if self.bus.hle_bios_active() {
                    self.cpu_cycles_remaining = self.bus.step_hle_bios().max(1);
                } else {
                    // Sample IRQ only at instruction boundaries; mid-instruction never samples.
                    // Falls through to the shared epilogue (decrement sets dispatch timing).
                    if !self.cpu.micro_pending() {
                        // Sample IRQ only at instruction boundaries; mid-instruction never samples.
                        // Falls through to the shared epilogue (decrement sets dispatch timing).
                        // Entry cost comes from service_irq (real refill waits + prologue count).
                        if let Some(irq_entry_cycles) = self.cpu.service_irq(&mut self.bus) {
                            self.cpu_cycles_remaining = irq_entry_cycles;
                        } else if let Some(acc) = self.drain_micro() {
                            self.cpu_cycles_remaining = acc.max(1) as u32;
                        } else {
                            // Unreachable: every instruction class expands,
                            // so the first drain always yields an op.
                            // Consume the tick safely.
                            self.cpu_cycles_remaining = 1;
                        }
                    } else if let Some(acc) = self.drain_micro() {
                        self.cpu_cycles_remaining = acc.max(1) as u32;
                    } else {
                        // Unreachable (queue was non-empty, so the first
                        // pop succeeds); consume the tick safely.
                        self.cpu_cycles_remaining = 1;
                    }
                }
            }
            self.cpu_cycles_remaining = self.cpu_cycles_remaining.saturating_sub(1);
        }
        self.tick = self.tick.wrapping_add(1);
        let frame_end = self.bus.tick();
        // Age the post-LDM^ bank-conflict window once per T-cycle.
        self.cpu.registers_mut().tick_ldm_conflict();
        // IntrWait wake-exit latency (see `wake_latency`): burn as
        // CPU-stall cycles so the staging IRQ line wins the race against
        // the woken thread. Subsumed by any longer in-flight charge.
        let wake_latency = self.bus.take_wake_latency();
        if wake_latency > 0 {
            self.cpu_cycles_remaining = self.cpu_cycles_remaining.max(wake_latency);
        }
        // DMA prefetch-collision arbitration (see `dma_stall_pending`):
        // unlike wake latency this serializes with in-flight work (the
        // bus arbitration cycle is extra, like Mesen's Step on Reset),
        // so it adds instead of maxing.
        let dma_stall = self.bus.take_dma_stall();
        if dma_stall > 0 {
            self.cpu_cycles_remaining += dma_stall;
        }
        frame_end
    }
}

impl Default for GbaSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::header::finalize_test_gba_rom;

    fn start_cpu_set(system: &mut GbaSystem, source: u32, destination: u32, len_mode: u32) {
        let registers = system.cpu.registers_mut();
        registers.set_r(0, source);
        registers.set_r(1, destination);
        registers.set_r(2, len_mode);
        crate::bios::handle_swi(registers, &mut system.bus, 0x0B);
    }

    #[test]
    fn step_tcycle_advances_exactly_one_cycle() {
        let mut system = GbaSystem::new();
        assert_eq!(system.tick, 0);
        system.step_tcycle();
        assert_eq!(system.tick, 1);
        assert!(system.cpu_cycles_remaining > 0);
    }

    #[test]
    fn halted_system_does_not_execute_cpu() {
        let mut system = GbaSystem::new();
        let pc = system.cpu.registers().pc();
        system.bus.enter_halt(1);
        for _ in 0..10 {
            system.step_tcycle();
        }
        assert_eq!(system.cpu.registers().pc(), pc);
    }

    #[test]
    fn hle_bios_operation_blocks_caller_until_complete() {
        let mut system = GbaSystem::new();
        system.bus.write32(0x03000000, 0x12345678);
        start_cpu_set(&mut system, 0x03000000, 0x03000004, (1 << 26) | 1);
        let caller_pc = system.cpu.registers().pc();

        while system.bus.hle_bios_active() {
            system.step_tcycle();
            assert_eq!(system.cpu.registers().pc(), caller_pc);
        }

        assert_eq!(system.bus.read32(0x03000004), 0x12345678);
    }

    #[test]
    fn hle_bios_operation_resumes_after_halt() {
        let mut system = GbaSystem::new();
        system.bus.write16(0x04000200, 1);
        system.bus.write32(0x03000000, 1);
        start_cpu_set(&mut system, 0x03000000, 0x04000300, (1 << 26) | 1);

        while !system.bus.is_halted() {
            system.step_tcycle();
        }
        assert!(system.bus.hle_bios_active());
        for _ in 0..4 {
            system.step_tcycle();
        }
        assert!(system.bus.hle_bios_active());

        system.bus.request_interrupt(1);
        while system.bus.hle_bios_active() {
            system.step_tcycle();
        }
        assert!(!system.bus.is_halted());
    }

    #[test]
    fn dma_preempts_hle_bios_transfer() {
        let mut system = GbaSystem::new();
        for index in 0..8 {
            system
                .bus
                .write32(0x03000000 + index * 4, 0x10000000 + index);
        }
        start_cpu_set(&mut system, 0x03000000, 0x03000040, (1 << 26) | 8);

        while system.bus.read32(0x03000040) == 0 {
            system.step_tcycle();
        }

        for index in 0..4 {
            system
                .bus
                .write32(0x03000100 + index * 4, 0xA0000000 + index);
        }
        system.bus.write32(0x040000D4, 0x03000100);
        system.bus.write32(0x040000D8, 0x02000000);
        system.bus.write32(0x040000DC, 0x84000004);

        while !system.bus.dma_active() {
            system.step_tcycle();
        }
        assert!(system.bus.dma_active());
        assert_eq!(system.bus.read32(0x03000040), 0x10000000);
        assert_eq!(system.bus.read32(0x0300005C), 0);

        while system.bus.dma_active() || system.bus.hle_bios_active() {
            system.step_tcycle();
        }
        assert_eq!(system.bus.read32(0x0200000C), 0xA0000003);
        for index in 0..8 {
            assert_eq!(
                system.bus.read32(0x03000040 + index * 4),
                0x10000000 + index
            );
        }
    }

    #[test]
    fn run_frame_advances_one_lcd_frame() {
        let mut system = GbaSystem::new();
        assert_eq!(
            system.run_frame().len(),
            crate::ppu::WIDTH * crate::ppu::HEIGHT
        );
        assert_eq!(system.tick, 280896);
    }

    #[test]
    fn test_rom_loader_allows_only_logo_mismatch() {
        let mut rom = vec![0; 0x200];
        finalize_test_gba_rom(&mut rom);
        rom[0x61] ^= 0x07;
        assert!(GbaSystem::from_rom(rom.clone()).is_none());
        assert!(GbaSystem::from_test_rom(rom.clone()).is_some());

        rom[0xB2] = 0;
        assert!(GbaSystem::from_test_rom(rom).is_none());
    }
}
