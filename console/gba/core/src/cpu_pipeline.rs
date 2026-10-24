use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// 3段パイプラインの初期充填とフラッシュヘルパー。
pub fn fill_pipeline(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, pipeline: &mut [u32; 2]) {
    let pc = regs.pc();
    if regs.cpsr_t() {
        pipeline[0] = bus.fetch16(pc) as u32;
        pipeline[1] = bus.fetch16(pc.wrapping_add(2)) as u32;
        regs.set_pc(pc.wrapping_add(4));
    } else {
        // ARM: 2 x 32bit
        if let Some(pair) = bus.fetch_iwram_arm_pair(pc) {
            *pipeline = pair;
        } else {
            pipeline[0] = bus.fetch32(pc);
            pipeline[1] = bus.fetch32(pc.wrapping_add(4));
        }
        regs.set_pc(pc.wrapping_add(8));
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

    #[test]
    fn iwram_pair_refill_matches_individual_fetches() {
        // Normal and mirrored reads, a wrap within IWRAM, and a final
        // word crossing out of the IWRAM region (generic fallback).
        for pc in [0x0300_0100, 0x0300_7FFC, 0x0300_FFFC, 0x03FF_FFFC] {
            let mut fast = GbaMemoryBus::new();
            let mut reference = GbaMemoryBus::new();
            for bus in [&mut fast, &mut reference] {
                bus.write32(0x0300_0100, 0xE1A0_0000);
                bus.write32(0x0300_7FFC, 0xE3A0_0001);
                bus.write32(0x0300_0000, 0xEAFF_FFFE);
                bus.take_access_wait_cycles();
            }
            let mut regs = CpuRegisters::post_bios();
            regs.set_pc(pc);
            let mut actual = [0; 2];
            fill_pipeline(&mut regs, &mut fast, &mut actual);
            let expected = [reference.fetch32(pc), reference.fetch32(pc.wrapping_add(4))];
            assert_eq!(actual, expected, "pc={pc:#010x}");
            assert_eq!(regs.pc(), pc.wrapping_add(8));
            let a = rmp_serde::to_vec_named(&fast.export_state().unwrap()).unwrap();
            let b = rmp_serde::to_vec_named(&reference.export_state().unwrap()).unwrap();
            assert_eq!(a, b, "bus state differs at pc={pc:#010x}");
        }
    }
}
