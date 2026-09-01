pub mod arm;
pub mod arm_opcodes;
pub mod thumb;
pub mod thumb_opcodes;

#[cfg(test)]
mod sonar_coverage_tests;

use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// GBA CPU (ARM7TDMI) — 3段パイプライン。
pub struct GbaCpu {
    regs: CpuRegisters,
    pipeline: [u32; 2],
}

impl GbaCpu {
    pub fn new() -> Self {
        Self {
            regs: CpuRegisters::post_bios(),
            pipeline: [0; 2],
        }
    }

    pub fn post_bios() -> Self {
        Self::new()
    }

    pub fn reset(&mut self, bus: &mut GbaMemoryBus) {
        self.regs = CpuRegisters::post_bios();
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        bus.take_access_wait_cycles();
    }

    pub fn registers(&self) -> &CpuRegisters {
        &self.regs
    }

    pub fn registers_mut(&mut self) -> &mut CpuRegisters {
        &mut self.regs
    }

    pub fn service_irq(&mut self, bus: &mut GbaMemoryBus) -> bool {
        if self.regs.cpsr() & (1 << 7) != 0 || !bus.irq_pending() {
            return false;
        }
        let return_address = self.regs.pc().wrapping_add(4);
        self.regs
            .enter_exception(0x12, 0x00000018, return_address, true);
        self.pipeline = [0; 2];
        bus.set_current_pc(0x00000018);
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
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
        cycles + bus.take_access_wait_cycles()
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
        if self.regs.take_pc_written() {
            self.pipeline = [0; 2];
            bus.set_current_pc(self.regs.pc());
            fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
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
            self.pipeline = [0; 2];
            bus.set_current_pc(self.regs.pc());
            fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
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
        bus.write16(0x04000200, 1 << 3);
        bus.write16(0x04000208, 1);
        bus.request_interrupt(1 << 3);
        assert!(cpu.service_irq(&mut bus));
        assert_eq!(cpu.regs.cpsr_mode(), 0x12);
        assert_ne!(cpu.regs.cpsr() & (1 << 7), 0);
        assert_eq!(cpu.regs.spsr() & 0x1F, 0x1F);
        assert_eq!(cpu.regs.pc(), 0x20);
        assert_eq!(cpu.regs.lr(), 0x08000004);
    }
}
