use crate::cpu_registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

/// Game Boy model variant for post-boot register initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GbcModel {
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
            GbcModel::Dmg => {
                // DMG post-boot register values
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
                // CGB post-boot: A=0x11, B=0x00, L=0x0D, Z=1
                cpu.registers.set_a(0x11);
                cpu.registers.set_f(0xB0);
                cpu.registers.set_b(0x00);
                cpu.registers.set_c(0x00);
                cpu.registers.set_d(0xFF);
                cpu.registers.set_e(0x56);
                cpu.registers.set_h(0x00);
                cpu.registers.set_l(0x0D);
            }
            GbcModel::Agb => {
                // AGB (GBA GBC mode): A=0x11, F=0x00, B=0x01, L=0x0D
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
