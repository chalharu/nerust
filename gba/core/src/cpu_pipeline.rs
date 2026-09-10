use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// 3段パイプラインの初期充填とフラッシュヘルパー。
pub fn fill_pipeline(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, pipeline: &mut [u32; 2]) {
    let pc = regs.pc();
    if regs.cpsr_t() {
        pipeline[0] = bus.fetch16(pc) as u32;
        pipeline[1] = bus.fetch16(pc + 2) as u32;
        regs.set_pc(pc + 4);
    } else {
        // ARM: 2 x 32bit
        pipeline[0] = bus.fetch32(pc);
        pipeline[1] = bus.fetch32(pc + 4);
        regs.set_pc(pc + 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn fill_pipeline_sets_pc_plus_8() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        let mut pipeline = [0u32; 2];
        fill_pipeline(&mut regs, &mut bus, &mut pipeline);
        assert_eq!(regs.pc(), 0x08000008);
    }

    #[test]
    fn fill_thumb_pipeline_sets_pc_plus_4() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_cpsr(regs.cpsr() | (1 << 5));
        let mut bus = GbaMemoryBus::new();
        let mut pipeline = [0u32; 2];
        fill_pipeline(&mut regs, &mut bus, &mut pipeline);
        assert_eq!(regs.pc(), 0x08000004);
    }
}
