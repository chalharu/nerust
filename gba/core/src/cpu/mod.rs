pub mod micro_op;
pub(crate) mod semantics;

use crate::cpu::micro_op::HLE_IRQ_RETURN_TRAMPOLINE;
use crate::cpu::micro_op::{
    AluEffect, BlockEmptyEffect, BlockEndEffect, BlockWord, MemAccess, MicroOp, MicroOpVec,
    PcRelRead,
};
use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

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
    /// Pending micro-ops of the in-flight instruction (per-cycle remodel
    /// slice 3b). Non-empty between the op-steps of one instruction;
    /// IRQ sampling and HLE entry only run at instruction boundaries
    /// (empty queue). Index-drained (`micro_pos`): the consumed prefix
    /// is never moved or revisited; `clear` on refill keeps the inline
    /// (or spilled block-transfer) capacity across instructions.
    micro_ops: MicroOpVec,
    micro_pos: usize,
}

impl GbaCpu {
    pub fn new() -> Self {
        Self {
            regs: CpuRegisters::post_bios(),
            pipeline: [0; 2],
            irq_return_stack: Vec::new(),
            micro_ops: MicroOpVec::new(),
            micro_pos: 0,
        }
    }

    pub fn post_bios() -> Self {
        Self::new()
    }

    pub fn reset(&mut self, bus: &mut GbaMemoryBus) {
        self.regs = CpuRegisters::post_bios();
        self.irq_return_stack.clear();
        self.micro_ops.clear();
        self.micro_pos = 0;
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        bus.take_access_wait_cycles();
    }

    /// True while an instruction's micro-ops are still draining.
    pub(crate) fn micro_pending(&self) -> bool {
        self.micro_pos < self.micro_ops.len()
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
        self.micro_ops.clear();
        self.micro_pos = 0;
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        bus.take_access_wait_cycles();
    }

    /// Take a pending IRQ: returns the entry charge (discarded source
    /// fetch + handler refill bus waits + skipped-BIOS prologue count),
    /// or None when no IRQ is taken.
    pub fn service_irq(&mut self, bus: &mut GbaMemoryBus) -> Option<u32> {
        if self.regs.cpsr() & (1 << 7) != 0 || !bus.irq_pending() {
            return None;
        }
        // Post-enable timer0 takes sample one boundary later (see
        // defer_timer0_take): skip this boundary, IF stays raised.
        if bus.defer_timer0_take() {
            return None;
        }
        // Per-cycle co-sim: at most one entry observes the halt-wake
        // edge (see `take_woke_from_halt`). The wake itself only parks
        // the pipeline; the IRQ line follows 2 ticks later (line_queue),
        // so the woken thread refills the pipe before entry and the
        // discarded in-flight read below still applies. `woke` only feeds
        // the T4 discount gate (a woken take never discounts).
        let woke = bus.take_woke_from_halt();
        // Interrupted PC/mode for the discarded read below (registers
        // change on exception entry).
        let src_pc = self.regs.pc();
        let src_thumb = self.regs.cpsr_t();
        // In-flight opcodes at the interrupted boundary (class-gated take
        // entry reads these, never addresses).
        let entry_opcode = self.pipeline[0];
        let entry_next = self.pipeline[1];
        // Take#1 grid proxy for the T11 gate (latest first-take wins).
        bus.record_take1_latency();
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
        // Discarded in-flight read at the interrupted PC: charged, result
        // dropped. The flush restarts the prefetch stream first, so this
        // is non-sequential (source-region N).
        if src_thumb {
            bus.fetch16(src_pc & !1);
        } else {
            bus.fetch32(src_pc & !3);
        }
        // First handler fetch evolved N (cold tags) but the word was
        // prefetched on the prologue's free ROM bus: refund N-S so the
        // refill costs S+S while the window keeps its natural tail.
        bus.credit_entry_refill(target);
        fill_pipeline(&mut self.regs, bus, &mut self.pipeline);
        // Entry accounting: the vector+refill bus waits above are the real
        // charge (handler-region dependent), plus the HLE-as-code BIOS
        // prologue (see `bios_irq_prologue_cycles`). No source-region term
        // beyond the discarded fetch.
        let entry_bus = bus.take_access_wait_cycles().max(0) as u32;
        // HLE-as-code base prologue (23 for the anchored IWRAM path).
        // The timer0 gates below are deltas against it.
        let base_prologue = bus.bios_irq_prologue_cycles(woke);
        // T4/T5/T7/T8/T10/T11/T12 timer0 entry gates (+-3/+1/raise+25/
        // raise+25/+24/+1iter/+2).
        let prologue = if bus.resampled_timer0_entry(entry_opcode, entry_next, src_thumb) {
            base_prologue + 2
        } else if bus.brokenpipe_timer0_entry(entry_opcode, entry_next, src_thumb) {
            base_prologue + 24
        } else if bus.history_timer0_entry(entry_opcode, entry_next, src_thumb) {
            base_prologue + 26
        } else if let Some(pro) = bus.late_timer0_entry(entry_opcode, entry_next, src_thumb) {
            pro
        } else if let Some(pro) = bus.catchup_timer0_entry() {
            pro
        } else if bus.discount_timer0_entry(woke) {
            base_prologue - 3
        } else if bus.retook_timer0_entry() {
            base_prologue + 1
        } else {
            base_prologue
        };
        Some(entry_bus + prologue)
    }

    /// 1命令実行し、消費T-cycleを返す。micro-op drainのみ
    /// （全命令クラスがexpandするためfallbackなし）。
    pub fn step(&mut self, bus: &mut GbaMemoryBus) -> u32 {
        // Micro-ops drain here atomically, including IRQ-handler steps;
        // trampoline returns land the HLE epilogue in retire.
        let is_thumb = self.regs.cpsr_t();
        let mut acc = 0i64;
        while let Some(c) = crate::cpu::micro_op::step_op(
            &mut self.regs,
            bus,
            &mut self.pipeline,
            &mut self.micro_ops,
            &mut self.micro_pos,
            is_thumb,
            &mut self.irq_return_stack,
        ) {
            acc += c;
            if !self.micro_pending() {
                break;
            }
        }
        acc.max(1) as u32
    }

    /// Single micro-op step for the system driver (per-op ticks). Returns
    /// the op's true cost without any floor; `None` only on an uncovered
    /// fill (queue untouched, caller falls back).
    pub(crate) fn step_op(&mut self, bus: &mut GbaMemoryBus) -> Option<i64> {
        let is_thumb = self.regs.cpsr_t();
        crate::cpu::micro_op::step_op(
            &mut self.regs,
            bus,
            &mut self.pipeline,
            &mut self.micro_ops,
            &mut self.micro_pos,
            is_thumb,
            &mut self.irq_return_stack,
        )
    }
}

impl Default for GbaCpu {
    fn default() -> Self {
        Self::new()
    }
}

/// Phase 10 wire state: registers, fetch pipeline, in-flight micro-ops and
/// the HLE IRQ return stack. `#[cfg(test)]` helpers never enter the DTO.
/// The op buffer stores only the not-yet-drained suffix (the consumed
/// prefix is history), so it round-trips as a plain op sequence exactly
/// like the old queue's remaining contents.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GbaCpuState {
    regs: CpuRegisters,
    pipeline: [u32; 2],
    irq_return_stack: Vec<(u32, [u32; 5])>,
    micro_queue: Vec<MicroOp>,
}

impl GbaCpuState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let mode = self.regs.cpsr() & 0x1F;
        if !matches!(mode, 0x10 | 0x11 | 0x12 | 0x13 | 0x17 | 0x1B | 0x1F) {
            return Err(format!("cpu: invalid mode bits {mode:#X}"));
        }
        // `set_pc` masks r15 (Thumb: !1, ARM: !3), so bit 0 is always clear
        // and bit 1 is clear outside Thumb.
        let pc = self.regs.r(15);
        if pc & 1 != 0 {
            return Err(format!("cpu: misaligned pc {pc:#X}"));
        }
        if !self.regs.cpsr_t() && pc & 2 != 0 {
            return Err(format!("cpu: misaligned ARM pc {pc:#X}"));
        }
        // Armed by `arm_ldm_conflict` to 2, aged one T-cycle at a time.
        if self.regs.ldm_conflict_window() > 2 {
            return Err(format!(
                "cpu: ldm_conflict out of window: {}",
                self.regs.ldm_conflict_window()
            ));
        }
        if self.irq_return_stack.len() > 32 {
            return Err(format!(
                "cpu: irq_return_stack too deep: {}",
                self.irq_return_stack.len()
            ));
        }
        if self.micro_queue.len() > 256 {
            return Err(format!(
                "cpu: micro_queue too long: {}",
                self.micro_queue.len()
            ));
        }
        for op in &self.micro_queue {
            validate_micro_op(op)?;
        }
        Ok(())
    }
}

fn validate_reg_idx(name: &str, idx: usize) -> Result<(), String> {
    if idx > 15 {
        return Err(format!("cpu: {name} register out of range: {idx}"));
    }
    Ok(())
}

fn validate_mem_access(access: &MemAccess) -> Result<(), String> {
    if !matches!(access.width, 1 | 2 | 4) {
        return Err(format!("cpu: bad access width {}", access.width));
    }
    validate_reg_idx("mem rd", access.rd)?;
    validate_reg_idx("mem rn", access.rn)?;
    if let Some(reg) = access.offset_register {
        validate_reg_idx("mem offset", reg)?;
    }
    Ok(())
}

fn validate_micro_op(op: &MicroOp) -> Result<(), String> {
    match op {
        MicroOp::MemRead(access) | MicroOp::MemWrite(access) => validate_mem_access(access),
        MicroOp::PcRelRead(PcRelRead { rd, .. }) => validate_reg_idx("pcrel rd", *rd),
        MicroOp::BlockWord(BlockWord { reg, .. }) => {
            if *reg > 15 {
                return Err(format!("cpu: block word register out of range: {reg}"));
            }
            Ok(())
        }
        MicroOp::BlockEnd(BlockEndEffect { writeback, .. }) => {
            if let Some((reg, _)) = writeback {
                validate_reg_idx("block writeback", *reg)?;
            }
            Ok(())
        }
        MicroOp::BlockEmpty(BlockEmptyEffect { writeback_reg, .. }) => {
            if let Some((reg, _)) = writeback_reg {
                validate_reg_idx("block empty writeback", *reg)?;
            }
            Ok(())
        }
        MicroOp::CommitAlu(AluEffect { rd, rn, .. }) => {
            validate_reg_idx("alu rd", *rd)?;
            validate_reg_idx("alu rn", *rn)
        }
        MicroOp::Internal
        | MicroOp::CommitDpReg(_)
        | MicroOp::CommitMul(_)
        | MicroOp::CommitSwp(_)
        | MicroOp::CommitPsr(_)
        | MicroOp::CommitThumb(_)
        | MicroOp::BlHigh(_)
        | MicroOp::BlLow(_)
        | MicroOp::Bx(_)
        | MicroOp::TrapSwi(_)
        | MicroOp::TrapUnd
        | MicroOp::TakenBranch(_)
        | MicroOp::BlockStart(_) => Ok(()),
    }
}

impl GbaCpu {
    pub(crate) fn export_state(&self) -> GbaCpuState {
        GbaCpuState {
            regs: self.regs.clone(),
            pipeline: self.pipeline,
            irq_return_stack: self.irq_return_stack.clone(),
            micro_queue: self
                .micro_ops
                .iter()
                .skip(self.micro_pos)
                .copied()
                .collect(),
        }
    }

    pub(crate) fn import_state(&mut self, state: GbaCpuState) -> Result<(), String> {
        state.validate()?;
        self.regs = state.regs;
        self.pipeline = state.pipeline;
        self.irq_return_stack = state.irq_return_stack;
        self.micro_ops.clear();
        self.micro_ops.extend(state.micro_queue);
        self.micro_pos = 0;
        Ok(())
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
        assert!(cpu.service_irq(&mut bus).is_some());
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

    #[test]
    fn cpu_state_round_trips_mid_instruction() {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let start = 0x02000000;
        bus.write32(start, 0xE8900007); // LDMIA R0,{R0-R2}: multi-op block
        bus.write32(start + 4, 0xE3A03009); // MOV R3,#9
        cpu.regs.set_pc(start);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        // Drain a single micro-op: the block transfer stays in flight.
        assert!(cpu.step_op(&mut bus).is_some());
        assert!(cpu.micro_pending());
        // A nested IRQ return slot is live as well.
        cpu.irq_return_stack.push((start + 8, [1, 2, 3, 4, 5]));

        let state = cpu.export_state();
        state.validate().unwrap();
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        let decoded: GbaCpuState = rmp_serde::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        let mut restored = GbaCpu::post_bios();
        restored.import_state(decoded).unwrap();
        let again = rmp_serde::to_vec_named(&restored.export_state()).unwrap();
        assert_eq!(bytes, again);
        assert!(
            restored
                .micro_ops
                .iter()
                .skip(restored.micro_pos)
                .any(|op| matches!(op, MicroOp::BlockWord(_)))
        );
        assert_eq!(restored.irq_return_stack.len(), 1);
    }

    #[test]
    fn cpu_state_rejects_garbage() {
        let cpu = GbaCpu::post_bios();
        let mut state = cpu.export_state();
        state.validate().unwrap();
        // Invalid mode bits.
        state.regs.set_cpsr(0x1C);
        assert!(state.validate().is_err());
        // Oversized queue.
        state = cpu.export_state();
        state
            .micro_queue
            .resize(300, crate::cpu::micro_op::MicroOp::Internal);
        assert!(state.validate().is_err());
        // Bad access width.
        state = cpu.export_state();
        state
            .micro_queue
            .push(MicroOp::MemRead(crate::cpu::micro_op::MemAccess {
                width: 3,
                rd: 0,
                rn: 0,
                offset: 0,
                offset_register: None,
                subtract: false,
                is_sp: false,
                signed_load: false,
                post_indexed: false,
                writeback: false,
                halfword_odd_quirk: false,
                store_value: None,
            }));
        assert!(state.validate().is_err());
    }
}
