use crate::cpu_registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CpuState {
    registers: CpuRegisters,
    phase: CpuPhaseState,
    ime_delayed: bool,
    ime_enable_armed: bool,
    opcode: u8,
    operands: [u8; 2],
    operand_count: u8,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum CpuPhaseState {
    FetchOpcode,
    ExecuteOpcode {
        step: u8,
    },
    InterruptDispatch {
        step: u8,
        pending_ie: u8,
        pending_if: u8,
    },
}

/// Game Boy model variant for post-boot register initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GbcModel {
    /// Original DMG-0 (early revision): distinct post-boot registers.
    Dmg0,
    /// Common DMG-CPU / MGB revision.
    Dmg,
    Cgb,
    Agb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepResult {
    Continue,
    Exit,
}

pub(crate) type HandlerFn = fn(&mut Lr35902Cpu, &mut GbcMemoryBus, u8) -> StepResult;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Phase {
    FetchOpcode,
    ExecuteOpcode {
        handler: HandlerFn,
        step: u8,
    },
    /// Real hardware takes 5 M-cycles to acknowledge and dispatch
    /// an interrupt (push PC, set PC to handler vector). The dispatch
    /// re-evaluates IE & IF after pushing PC (the push can write to the
    /// IE/IF registers, cancelling or changing the dispatch — mooneye ie_push).
    InterruptDispatch {
        step: u8,
        /// IE snapshot taken after the high-byte PC push.
        pending_ie: u8,
        /// IF value used for the dispatch decision (old IF if the low-byte
        /// push targets the IF register).
        pending_if: u8,
    },
}

pub struct Lr35902Cpu {
    registers: CpuRegisters,
    phase: Phase,
    ime_delayed: bool,
    ime_enable_armed: bool,
    opcode: u8,
    operands: [u8; 2],
    operand_count: u8,
}

impl Lr35902Cpu {
    pub(crate) fn export_state(&self) -> CpuState {
        let phase = match self.phase {
            Phase::FetchOpcode => CpuPhaseState::FetchOpcode,
            Phase::ExecuteOpcode { step, .. } => CpuPhaseState::ExecuteOpcode { step },
            Phase::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            } => CpuPhaseState::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            },
        };
        CpuState {
            registers: self.registers,
            phase,
            ime_delayed: self.ime_delayed,
            ime_enable_armed: self.ime_enable_armed,
            opcode: self.opcode,
            operands: self.operands,
            operand_count: self.operand_count,
        }
    }

    pub(crate) fn import_state(&mut self, state: CpuState) -> Result<(), String> {
        state.registers.validate()?;
        if state.operand_count > 2 {
            return Err("CPU operand count exceeds two bytes".into());
        }
        let phase = match state.phase {
            CpuPhaseState::FetchOpcode => Phase::FetchOpcode,
            CpuPhaseState::ExecuteOpcode { step } => {
                if !(1..=8).contains(&step) {
                    return Err(format!("invalid CPU opcode step: {step}"));
                }
                let handler = crate::cpu_opcodes::handler_table()[state.opcode as usize];
                Phase::ExecuteOpcode { handler, step }
            }
            CpuPhaseState::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            } => {
                if !(1..=4).contains(&step) {
                    return Err(format!("invalid CPU interrupt dispatch step: {step}"));
                }
                Phase::InterruptDispatch {
                    step,
                    pending_ie,
                    pending_if,
                }
            }
        };
        self.registers = state.registers;
        self.phase = phase;
        self.ime_delayed = state.ime_delayed;
        self.ime_enable_armed = state.ime_enable_armed;
        self.opcode = state.opcode;
        self.operands = state.operands;
        self.operand_count = state.operand_count;
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            registers: CpuRegisters::new(),
            phase: Phase::FetchOpcode,
            ime_delayed: false,
            ime_enable_armed: false,
            opcode: 0,
            operands: [0; 2],
            operand_count: 0,
        }
    }

    /// Create CPU with post-boot register values for a specific model.
    pub fn with_model(model: GbcModel) -> Self {
        let mut cpu = Self::new();
        match model {
            GbcModel::Dmg0 => {
                // DMG-0 post-boot register values (F=$00, B=$FF, E=$C1,
                // H=$84, L=$03).
                cpu.registers.set_a(0x01);
                cpu.registers.set_f(0x00);
                cpu.registers.set_b(0xFF);
                cpu.registers.set_c(0x13);
                cpu.registers.set_d(0x00);
                cpu.registers.set_e(0xC1);
                cpu.registers.set_h(0x84);
                cpu.registers.set_l(0x03);
            }
            GbcModel::Dmg => {
                // Common DMG-CPU / MGB post-boot register values
                // (F=$B0, B=$00, E=$D8, H=$01, L=$4D).
                cpu.registers.set_a(0x01);
                cpu.registers.set_f(0xB0);
                cpu.registers.set_b(0x00);
                cpu.registers.set_c(0x13);
                cpu.registers.set_d(0x00);
                cpu.registers.set_e(0xD8);
                cpu.registers.set_h(0x01);
                cpu.registers.set_l(0x4D);
            }
            GbcModel::Cgb => {
                // CGB post-boot (CGB-native game): A=$11, F=$80, D=$FF,
                // E=$56, L=$0D.
                cpu.registers.set_a(0x11);
                cpu.registers.set_f(0x80);
                cpu.registers.set_b(0x00);
                cpu.registers.set_c(0x00);
                cpu.registers.set_d(0xFF);
                cpu.registers.set_e(0x56);
                cpu.registers.set_h(0x00);
                cpu.registers.set_l(0x0D);
            }
            GbcModel::Agb => {
                // AGB (GBA GBC mode): A=$11, F=$00, B=$01, L=$0D
                cpu.registers.set_a(0x11);
                cpu.registers.set_f(0x00);
                cpu.registers.set_b(0x01);
                cpu.registers.set_c(0x00);
                cpu.registers.set_d(0xFF);
                cpu.registers.set_e(0x56);
                cpu.registers.set_h(0x00);
                cpu.registers.set_l(0x0D);
            }
        }
        cpu.registers.set_sp(0xFFFE);
        cpu.registers.set_pc(0x0100);
        cpu
    }

    /// Override the CGB post-boot registers for a DMG-compatible game
    /// (cgb_flag bit 7 clear): the CGB boot ROM initialises D=$00, E=$08 and
    /// HL=$007C (or $991A) instead of the CGB-native D=$FF, E=$56, L=$0D.
    pub fn set_cgb_dmg_mode_registers(&mut self) {
        self.registers.set_d(0x00);
        self.registers.set_e(0x08);
        self.registers.set_h(0x00);
        self.registers.set_l(0x7C);
    }

    #[inline]
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }
    #[inline]
    pub(crate) fn set_phase(&mut self, p: Phase) {
        self.phase = p;
    }
    #[inline]
    pub(crate) fn ime_delayed(&self) -> bool {
        self.ime_delayed
    }
    #[inline]
    pub(crate) fn set_ime_delayed(&mut self, v: bool) {
        self.ime_delayed = v;
    }
    #[inline]
    pub(crate) fn arm_delayed_ime(&mut self) {
        if self.ime_delayed {
            self.ime_delayed = false;
            self.ime_enable_armed = true;
        }
    }
    #[inline]
    pub(crate) fn take_armed_ime(&mut self) -> bool {
        std::mem::take(&mut self.ime_enable_armed)
    }
    #[inline]
    pub(crate) fn cancel_delayed_ime(&mut self) {
        self.ime_delayed = false;
        self.ime_enable_armed = false;
    }
    #[inline]
    pub fn registers(&self) -> &CpuRegisters {
        &self.registers
    }
    #[inline]
    pub fn registers_mut(&mut self) -> &mut CpuRegisters {
        &mut self.registers
    }
    #[inline]
    pub(crate) fn opcode(&self) -> u8 {
        self.opcode
    }
    #[inline]
    pub(crate) fn set_opcode(&mut self, v: u8) {
        self.opcode = v;
    }
    #[inline]
    pub(crate) fn operand(&self, idx: usize) -> u8 {
        self.operands[idx]
    }
    #[inline]
    pub(crate) fn set_operand(&mut self, idx: usize, v: u8) {
        self.operands[idx] = v;
    }
    #[inline]
    pub(crate) fn operand_count(&self) -> u8 {
        self.operand_count
    }
    #[inline]
    pub(crate) fn set_operand_count(&mut self, v: u8) {
        self.operand_count = v;
    }

    pub(crate) fn pc_read(&mut self, bus: &mut GbcMemoryBus) -> u8 {
        let pc = self.registers.pc();
        self.registers.set_pc(pc.wrapping_add(1));
        bus.read(pc)
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    #[test]
    fn state_round_trip_rebuilds_opcode_handler() {
        let mut source = Lr35902Cpu::new();
        source.opcode = 0xCD;
        source.operands = [0x34, 0x12];
        source.operand_count = 2;
        source.phase = Phase::ExecuteOpcode {
            handler: crate::cpu_opcodes::handler_table()[0xCD],
            step: 2,
        };
        let bytes = rmp_serde::to_vec_named(&source.export_state()).unwrap();
        let state: CpuState = rmp_serde::from_slice(&bytes).unwrap();
        let mut restored = Lr35902Cpu::new();
        restored.import_state(state).unwrap();

        assert_eq!(restored.opcode, 0xCD);
        assert_eq!(restored.operands, [0x34, 0x12]);
        assert!(matches!(
            restored.phase,
            Phase::ExecuteOpcode { step: 2, .. }
        ));
    }

    #[test]
    fn state_rejects_invalid_execute_step_without_mutation() {
        let mut target = Lr35902Cpu::new();
        let before = target.registers;
        let mut state = target.export_state();
        state.phase = CpuPhaseState::ExecuteOpcode { step: 0 };
        assert!(target.import_state(state).is_err());
        assert_eq!(target.registers, before);
    }
}
