use crate::cpu_registers::CpuRegisters;
use crate::interrupt::InterruptKind;
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
    ExecuteOpcode { handler: HandlerFn, step: u8 },
}

pub struct Lr35902Cpu {
    registers: CpuRegisters,
    phase: Phase,
    ime_delayed: bool,
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
                // CGB post-boot: A=0x11, B=0x00, Z=1
                cpu.registers.set_a(0x11);
                cpu.registers.set_f(0xB0);
                cpu.registers.set_b(0x00);
                cpu.registers.set_c(0x00);
                cpu.registers.set_d(0xFF);
                cpu.registers.set_e(0x56);
                cpu.registers.set_h(0x00);
                cpu.registers.set_l(0x00);
            }
            GbcModel::Agb => {
                // AGB (GBA GBC mode): A=0x11, B=0x01, L=0x0D, Z=1
                cpu.registers.set_a(0x11);
                cpu.registers.set_f(0xB0);
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

/// Push PC to stack and jump to interrupt vector.
pub(crate) fn dispatch_interrupt(
    regs: &mut CpuRegisters,
    kind: InterruptKind,
    bus: &mut GbcMemoryBus,
) {
    let sp = regs.sp();
    regs.set_sp(sp.wrapping_sub(1));
    bus.write(regs.sp(), (regs.pc() >> 8) as u8);
    let sp = regs.sp();
    regs.set_sp(sp.wrapping_sub(1));
    bus.write(regs.sp(), regs.pc() as u8);
    regs.set_pc(kind.vector());
}
