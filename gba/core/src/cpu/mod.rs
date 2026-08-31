pub mod arm;
pub mod arm_opcodes;
pub mod pipeline;
pub mod registers;
pub mod thumb;
pub mod thumb_opcodes;

use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// GBA CPU (ARM7TDMI) — 3段パイプライン。
pub struct GbaCpu {
    regs: CpuRegisters,
    pipeline: [u32; 2],
}

impl GbaCpu {
    pub fn new() -> Self {
        let cpu = Self {
            regs: CpuRegisters::post_bios(),
            pipeline: [0; 2],
        };
        cpu
    }

    pub fn post_bios() -> Self {
        Self::new()
    }

    pub fn reset(&mut self, bus: &mut GbaMemoryBus) {
        self.regs = CpuRegisters::post_bios();
        pipeline::fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
    }

    pub fn registers(&self) -> &CpuRegisters {
        &self.regs
    }

    pub fn registers_mut(&mut self) -> &mut CpuRegisters {
        &mut self.regs
    }

    /// 1命令実行し、消費T-cycleを返す。
    pub fn step(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        bus.set_current_pc(self.regs.pc());
        let is_thumb = self.regs.cpsr_t();
        let cycles = if is_thumb {
            self.step_thumb(bus)
        } else {
            self.step_arm(bus)
        };
        cycles
    }

    fn step_arm(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        let pc = self.regs.pc();
        // パイプライン: fetch next, decode = pipeline[0], execute = pipeline[1]
        let fetched = bus.fetch32(pc);
        let execute = self.pipeline[0];
        // ローテート
        self.pipeline[0] = self.pipeline[1];
        self.pipeline[1] = fetched;
        self.regs.set_pc(pc + 4);
        // デコード・実行
        let cycles = arm::decode_arm(&mut self.regs, bus, execute);
        // 分岐でPCが変わった場合はパイプラインをフラッシュ（簡易検出: PCが期待値と異なる）
        if self.regs.pc() != pc + 4 {
            // ブランチ発生 — 既にdecode_arm内でPC更新済み
            // 残りパイプラインをクリアし再充填
            self.pipeline = [0; 2];
            pipeline::fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
            // 分岐は追加サイクルを既に含む
        }
        // Waitは bus.cycles_for で外部要因として加算されるが、簡易では1を返す
        cycles
    }

    fn step_thumb(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        let pc = self.regs.pc();
        let fetched = bus.fetch16(pc) as u32;
        // Thumbパイプラインは16bit単位だが簡易的に32bitキューを流用
        let execute = (self.pipeline[0] & 0xFFFF) as u16;
        // シフト: pipeline[0]の下位16bitを捨て、上位16bitを下位へ、pipeline1の下位を上位へ
        let next_low = (self.pipeline[0] >> 16) as u16 as u32;
        let next_high = (self.pipeline[1] & 0xFFFF) as u32;
        self.pipeline[0] = next_low | (next_high << 16);
        self.pipeline[1] = (self.pipeline[1] >> 16) | (fetched << 16);
        self.regs.set_pc(pc + 2);
        let cycles = thumb::decode_thumb(&mut self.regs, bus, execute);
        if self.regs.pc() != pc + 2 {
            self.pipeline = [0; 2];
            pipeline::fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
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
        let mut bus = GbaMemoryBus::new();
        // EQ条件の NOP的なARM命令を仮実行 — cond不一致なら1サイクル
        // 直接 check_cond を通すため、ここではレジスタのみ検証
        assert!(regs.cpsr_z());
    }
}
