pub mod arm;
pub mod arm_opcodes;
pub mod micro_op;
pub mod thumb;
pub mod thumb_opcodes;

#[cfg(test)]
mod sonar_coverage_tests;

use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

const HLE_IRQ_RETURN_TRAMPOLINE: u32 = 0x00000014;

/// Real-BIOS IRQ epilogue cost (fitted): the HLE return trampoline skips
/// the register-restore + exception-return sequence the real BIOS runs
/// after the user handler on HW. Fitted at 7 against the mgba-suite
/// timers dispatch rounds (cascade sums need the longer pause while the
/// frozen 1d1i exit values prefer ~5; 7 is the least-bad compromise --
/// see the timers note in rom_tests.yaml). Pin-verified: timer-irq fail
/// sets identical, cancel-ime/if + timer reload/start-stop/tick-before-
/// reload pass, Timing and misc_edge unchanged. Recalibrate against
/// those if this changes.
const HLE_IRQ_EPILOGUE_CYCLES: u32 = 7;

/// GBA CPU (ARM7TDMI) — 3段パイプライン。
pub struct GbaCpu {
    regs: CpuRegisters,
    pipeline: [u32; 2],
    /// HLE IRQ return slots, innermost last. The real BIOS prologue nests
    /// through the IRQ stack, so a second dispatch inside a handler must
    /// not clobber the outer return (mgba-suite timers: the master
    /// re-enables IRQ (MSR SYS+clear-I) while an every-tick timer keeps
    /// the line asserted, nesting deeply until testIrq stops it).
    irq_return_stack: Vec<(u32, [u32; 5])>,
}

impl GbaCpu {
    pub fn new() -> Self {
        Self {
            regs: CpuRegisters::post_bios(),
            pipeline: [0; 2],
            irq_return_stack: Vec::new(),
        }
    }

    pub fn post_bios() -> Self {
        Self::new()
    }

    pub fn reset(&mut self, bus: &mut GbaMemoryBus) {
        self.regs = CpuRegisters::post_bios();
        self.irq_return_stack.clear();
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        bus.take_access_wait_cycles();
    }

    pub fn registers(&self) -> &CpuRegisters {
        &self.regs
    }

    pub fn registers_mut(&mut self) -> &mut CpuRegisters {
        &mut self.regs
    }

    /// Test-only jump: set PC/mode registers and refill the pipeline so that
    /// hijacked ROM code executes faithfully (used by DMA timing fits).
    #[cfg(test)]
    pub(crate) fn test_jump(&mut self, bus: &mut GbaMemoryBus, pc: u32, thumb: bool) {
        let mut cpsr = self.regs.cpsr();
        if thumb {
            cpsr |= 1 << 5;
        } else {
            cpsr &= !(1 << 5);
        }
        self.regs.set_cpsr(cpsr);
        self.regs.set_pc(pc);
        self.pipeline = [0; 2];
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        bus.take_access_wait_cycles();
    }

    pub fn service_irq(&mut self, bus: &mut GbaMemoryBus) -> bool {
        if self.regs.cpsr() & (1 << 7) != 0 || !bus.irq_pending() {
            return false;
        }
        let vector = bus.read32(0x03007FFC);
        // The real BIOS jumps to [03007FFCh] blindly; a handler can live in
        // any executable memory (IWRAM/EWRAM, any ROM mirror 08-0D, SRAM).
        // Only a null vector (nothing installed yet) falls back to the
        // exception vector, preserving boot-time behavior.
        let target = if vector != 0 { vector } else { 0x00000018 };
        let resume_address = self
            .regs
            .pc()
            .wrapping_sub(if self.regs.cpsr_t() { 4 } else { 8 });
        let exception_return_address = resume_address.wrapping_add(4);
        self.regs
            .enter_exception(0x12, target, exception_return_address, true);
        // The real BIOS IRQ prologue runs at 0x128+ before reaching the
        // user vector; its last fetched opcode (0xE25EF004) is what a
        // protected BIOS read observes during the ISR (jsmolka t003).
        bus.set_bios_prefetch(0xE25EF004);
        if target != 0x00000018 {
            self.irq_return_stack.push((
                resume_address,
                [
                    self.regs.r(0),
                    self.regs.r(1),
                    self.regs.r(2),
                    self.regs.r(3),
                    self.regs.r(12),
                ],
            ));
            self.regs.set_lr(HLE_IRQ_RETURN_TRAMPOLINE);
        }
        self.pipeline = [0; 2];
        bus.set_current_pc(target);
        bus.invalidate_prefetch_for_dma(target);
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        // Single-source IRQ entry accounting: the vector+refill bus waits
        // above are intentionally discarded here (not leaked into the next
        // step). The HLE entry cost IRQ_ENTRY_CYCLES + (N-1) in system.rs is
        // the sole charge, so entry is location-independent by design;
        // sub-tick entry overlap is future research (see rom_tests.yaml).
        bus.take_access_wait_cycles();
        true
    }

    /// 1命令実行し、消費T-cycleを返す。
    pub fn step(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        bus.take_access_wait_cycles();
        bus.set_current_pc(self.regs.pc());
        let is_thumb = self.regs.cpsr_t();
        let cycles = if is_thumb {
            self.step_thumb(bus)
        } else {
            self.step_arm(bus)
        };
        // Signed bus take: prefetch erases drive the accumulator negative
        // mid-instruction; the per-instruction net plus the base stays
        // positive (clamped at 1, like mGBA's per-instruction currentCycles
        // floor of the 1-cycle prefetch base).
        (cycles as i64 + bus.take_access_wait_cycles()).max(1) as u32
    }

    fn step_arm(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        let pc = self.regs.pc();
        // PCは実行中命令+8。pipeline[0]を実行し、pipeline[1]を次へ送る。
        let fetched = bus.fetch32(pc);
        let execute = self.pipeline[0];
        self.pipeline[0] = self.pipeline[1];
        self.pipeline[1] = fetched;
        self.regs.clear_pc_written();
        let cycles = arm::decode_arm(&mut self.regs, bus, execute);
        let pc_written = self.regs.take_pc_written();
        if pc_written {
            // True when this pc-write returns from a user IRQ handler
            // through the HLE trampoline (see HLE_IRQ_EPILOGUE_CYCLES).
            let mut irq_epilogue = 0;
            if self.regs.pc() == HLE_IRQ_RETURN_TRAMPOLINE
                && let Some((return_address, saved)) = self.irq_return_stack.pop()
            {
                self.regs.set_cpsr(self.regs.spsr());
                for (register, value) in [0, 1, 2, 3, 12].into_iter().zip(saved) {
                    self.regs.set_r(register, value);
                }
                // IRQ round-trip complete: the BIOS epilogue's last opcode
                // (0xE55EC002) is latched for protected reads (jsmolka t004).
                bus.set_bios_prefetch(0xE55EC002);
                self.regs.set_pc(return_address);
                irq_epilogue = HLE_IRQ_EPILOGUE_CYCLES;
            }
            self.pipeline = [0; 2];
            bus.set_current_pc(self.regs.pc());
            bus.invalidate_prefetch_for_dma(self.regs.pc());
            fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
            return cycles + irq_epilogue;
        } else {
            self.regs.set_pc(pc.wrapping_add(4));
        }
        cycles
    }

    fn step_thumb(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        let pc = self.regs.pc();
        let fetched = bus.fetch16(pc) as u32;
        let execute = (self.pipeline[0] & 0xFFFF) as u16;
        self.pipeline[0] = self.pipeline[1];
        self.pipeline[1] = fetched;
        self.regs.clear_pc_written();
        let cycles = thumb::decode_thumb(&mut self.regs, bus, execute);
        if self.regs.take_pc_written() {
            // Thumb user handlers return through the same trampoline (the
            // ARM side has handled it all along; the Thumb side previously
            // lacked the check). Same epilogue charge as ARM.
            let mut irq_epilogue = 0;
            if self.regs.pc() == HLE_IRQ_RETURN_TRAMPOLINE
                && let Some((return_address, saved)) = self.irq_return_stack.pop()
            {
                self.regs.set_cpsr(self.regs.spsr());
                for (register, value) in [0, 1, 2, 3, 12].into_iter().zip(saved) {
                    self.regs.set_r(register, value);
                }
                bus.set_bios_prefetch(0xE55EC002);
                self.regs.set_pc(return_address);
                irq_epilogue = HLE_IRQ_EPILOGUE_CYCLES;
            }
            self.pipeline = [0; 2];
            bus.set_current_pc(self.regs.pc());
            bus.invalidate_prefetch_for_dma(self.regs.pc());
            fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
            return cycles + irq_epilogue;
        } else {
            self.regs.set_pc(pc.wrapping_add(2));
        }
        cycles
    }
}

impl Default for GbaCpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn post_bios_registers() {
        let cpu = GbaCpu::post_bios();
        assert_eq!(cpu.registers().pc(), 0x08000000);
        assert_eq!(cpu.registers().sp(), 0x03007F00);
        assert_eq!(cpu.registers().cpsr() & 0x1F, 0x1F);
    }

    #[test]
    fn step_advances_pc() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        cpu.reset(&mut bus);
        let pc_before = cpu.registers().pc();
        cpu.step(&mut bus);
        assert_ne!(cpu.registers().pc(), pc_before);
    }

    #[test]
    fn cond_eq() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 30)); // Z=1
        assert!(regs.cpsr_z());
    }

    #[test]
    fn arm_pipeline_executes_in_order_and_counts_waits() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let start = 0x02000000;
        bus.write32(start, 0xE3A00001); // MOV R0,#1
        bus.write32(start + 4, 0xE3A01002); // MOV R1,#2
        bus.write32(start + 8, 0xE3A02003); // MOV R2,#3
        cpu.regs.set_pc(start);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();

        assert_eq!(cpu.step(&mut bus), 6); // 1 CPU + 5 EWRAM wait
        assert_eq!(cpu.regs.r(0), 1);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r(1), 2);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r(2), 3);
    }

    #[test]
    fn thumb_pipeline_executes_each_halfword_once() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let start = 0x03000000;
        bus.write16(start, 0x2001); // MOV R0,#1
        bus.write16(start + 2, 0x2102); // MOV R1,#2
        bus.write16(start + 4, 0x2203); // MOV R2,#3
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_pc(start);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();

        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!((cpu.regs.r(0), cpu.regs.r(1), cpu.regs.r(2)), (1, 2, 3));
    }

    #[test]
    fn branch_flushes_even_when_target_equals_architectural_pc() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let start = 0x03000000;
        bus.write32(start, 0xEA000000); // B to start+8
        bus.write32(start + 4, 0xE3A00001); // skipped
        bus.write32(start + 8, 0xE3A00002); // target
        cpu.regs.set_pc(start);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();

        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r(0), 2);
    }

    #[test]
    fn irq_enters_vector_with_banked_state() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let start = 0x02000000;
        bus.write32(start, 0xE3A00001); // MOV R0,#1
        cpu.regs.set_pc(start);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        bus.write16(0x04000200, 1 << 3);
        bus.write16(0x04000208, 1);
        bus.request_interrupt(1 << 3);
        bus.write32(0x03007FFC, 0x03000000);
        bus.write32(0x03000000, 0xE3A00002); // MOV R0,#2
        bus.write32(0x03000004, 0xE1A0_F00E); // MOV PC,LR
        // Let the delayed interrupt pipeline propagate (apply +1,
        // availability +1, CPU line +2) before sampling IRQ entry.
        for _ in 0..4 {
            bus.tick();
        }
        assert!(cpu.service_irq(&mut bus));
        assert_eq!(cpu.regs.cpsr_mode(), 0x12);
        assert_ne!(cpu.regs.cpsr() & (1 << 7), 0);
        assert_eq!(cpu.regs.spsr() & 0x1F, 0x1F);
        assert_eq!(cpu.regs.pc(), 0x03000008);
        assert_eq!(cpu.regs.lr(), HLE_IRQ_RETURN_TRAMPOLINE);

        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r(0), 2);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.cpsr_mode(), 0x1F);
        assert_eq!(cpu.regs.pc(), start + 8);
        assert_eq!(cpu.regs.r(0), 0);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.r(0), 1);
    }
}
