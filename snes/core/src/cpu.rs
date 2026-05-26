use crate::bus::CpuBus;

pub(crate) const RESET_CYCLES: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CpuFault {
    UnsupportedOpcode { opcode: u8, bank: u8, address: u16 },
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CpuStatus: u8 {
        const CARRY = 0b0000_0001;
        const ZERO = 0b0000_0010;
        const IRQ_DISABLE = 0b0000_0100;
        const DECIMAL = 0b0000_1000;
        const INDEX_8BIT = 0b0001_0000;
        const ACCUMULATOR_8BIT = 0b0010_0000;
        const OVERFLOW = 0b0100_0000;
        const NEGATIVE = 0b1000_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registers {
    pc: u16,
    pb: u8,
    db: u8,
    s: u16,
    d: u16,
    a: u16,
    x: u16,
    y: u16,
    p: CpuStatus,
    e: bool,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            pc: 0,
            pb: 0,
            db: 0,
            s: 0x01FF,
            d: 0,
            a: 0,
            x: 0,
            y: 0,
            p: CpuStatus::IRQ_DISABLE | CpuStatus::INDEX_8BIT | CpuStatus::ACCUMULATOR_8BIT,
            e: true,
        }
    }
}

impl Registers {
    pub fn pc(&self) -> u16 {
        self.pc
    }

    pub fn pb(&self) -> u8 {
        self.pb
    }

    pub fn db(&self) -> u8 {
        self.db
    }

    pub fn s(&self) -> u16 {
        self.s
    }

    pub fn d(&self) -> u16 {
        self.d
    }

    pub fn a(&self) -> u16 {
        self.a
    }

    pub fn x(&self) -> u16 {
        self.x
    }

    pub fn y(&self) -> u16 {
        self.y
    }

    pub fn status(&self) -> CpuStatus {
        self.p
    }

    pub fn emulation_mode(&self) -> bool {
        self.e
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuState {
    #[default]
    Resetting,
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImpliedOp {
    Nop,
    Clc,
    Cld,
    Cli,
    Clv,
    Sec,
    Sed,
    Sei,
    IncA,
    DecA,
    Inx,
    Iny,
    Dex,
    Dey,
    AslAcc,
    RolAcc,
    RorAcc,
    LsrAcc,
    Tcd,
    Tsc,
    Xce,
    Txs,
    Stp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Immediate8Op {
    Rep,
    Sep,
    Wdm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Immediate16Op {
    Pea,
    Per,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmediateLoadTarget {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmediateMathOp {
    BitA,
    AndA,
    OraA,
    EorA,
    AdcA,
    CmpA,
    CmpX,
    CmpY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShiftOp {
    Asl,
    Rol,
    Ror,
    Lsr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockMoveDirection {
    Increment,
    Decrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbsoluteOp {
    Adc {
        wide: bool,
    },
    AdcIndexedX {
        wide: bool,
    },
    AdcIndexedY {
        wide: bool,
    },
    And {
        wide: bool,
    },
    AndIndexedX {
        wide: bool,
    },
    AndIndexedY {
        wide: bool,
    },
    Ora {
        wide: bool,
    },
    OraIndexedX {
        wide: bool,
    },
    OraIndexedY {
        wide: bool,
    },
    Eor {
        wide: bool,
    },
    EorIndexedX {
        wide: bool,
    },
    EorIndexedY {
        wide: bool,
    },
    Inc {
        indexed_x: bool,
        wide: bool,
    },
    CmpA {
        wide: bool,
    },
    CmpAIndexedX {
        wide: bool,
    },
    CmpAIndexedY {
        wide: bool,
    },
    Dec {
        indexed_x: bool,
        wide: bool,
    },
    Cpx {
        wide: bool,
    },
    Cpy {
        wide: bool,
    },
    Ldx {
        wide: bool,
    },
    LdxIndexedY {
        wide: bool,
    },
    Ldy {
        wide: bool,
    },
    LdyIndexedX {
        wide: bool,
    },
    Lda {
        wide: bool,
    },
    LdaIndexedX {
        wide: bool,
    },
    LdaIndexedY {
        wide: bool,
    },
    Sta {
        wide: bool,
    },
    Sty {
        wide: bool,
    },
    Stz {
        wide: bool,
    },
    Bit {
        indexed_x: bool,
        wide: bool,
    },
    Shift {
        op: ShiftOp,
        indexed_x: bool,
        wide: bool,
    },
    Jmp,
    JmpIndirect,
    JmpIndexedXIndirect,
    JmlIndirect,
    Jsr,
    JsrIndexedXIndirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectOp {
    Adc {
        wide: bool,
    },
    AdcIndexedX {
        wide: bool,
    },
    And {
        wide: bool,
    },
    AndIndexedX {
        wide: bool,
    },
    Ora {
        wide: bool,
    },
    OraIndexedX {
        wide: bool,
    },
    Eor {
        wide: bool,
    },
    EorIndexedX {
        wide: bool,
    },
    Inc {
        indexed_x: bool,
        wide: bool,
    },
    CmpA {
        wide: bool,
    },
    CmpAIndexedX {
        wide: bool,
    },
    Bit {
        indexed_x: bool,
        wide: bool,
    },
    Shift {
        op: ShiftOp,
        indexed_x: bool,
        wide: bool,
    },
    Lda {
        wide: bool,
    },
    Sta {
        wide: bool,
    },
    Stx {
        wide: bool,
    },
    Sty {
        wide: bool,
    },
    Dec {
        indexed_x: bool,
        wide: bool,
    },
    Ldx {
        wide: bool,
    },
    LdxIndexedY {
        wide: bool,
    },
    Ldy {
        wide: bool,
    },
    LdyIndexedX {
        wide: bool,
    },
    LdaIndexedX {
        wide: bool,
    },
    Cpx {
        wide: bool,
    },
    Cpy {
        wide: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectIndexedIndirectOp {
    AdcA,
    AndA,
    OraA,
    EorA,
    CmpA,
    Lda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectIndirectOp {
    AdcA,
    AndA,
    OraA,
    EorA,
    CmpA,
    Lda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectIndirectLongOp {
    AdcA,
    AndA,
    OraA,
    EorA,
    CmpA,
    Lda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectIndirectIndexedYOp {
    AdcA,
    AdcALong,
    AndA,
    AndALong,
    OraA,
    OraALong,
    EorA,
    EorALong,
    CmpA,
    CmpALong,
    Lda,
    LdaLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackRelativeOp {
    AdcA,
    AndA,
    OraA,
    EorA,
    CmpA,
    Lda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackRelativeIndirectIndexedYOp {
    AdcA,
    AndA,
    OraA,
    EorA,
    CmpA,
    Lda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbsoluteLongOp {
    Adc { wide: bool },
    AdcIndexedX { wide: bool },
    And { wide: bool },
    AndIndexedX { wide: bool },
    Ora { wide: bool },
    OraIndexedX { wide: bool },
    Eor { wide: bool },
    EorIndexedX { wide: bool },
    CmpA { wide: bool },
    CmpAIndexedX { wide: bool },
    Lda { wide: bool },
    LdaIndexedX { wide: bool },
    Sta { wide: bool },
    Jml,
    Jsl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackOp {
    Pha,
    Pla,
    Php,
    Plp,
    Phx,
    Phy,
    Phb,
    Phk,
    Phd,
    Plx,
    Ply,
    Plb,
    Pld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchKind {
    Always,
    CarryClear,
    CarrySet,
    Equal,
    NotEqual,
    Minus,
    Plus,
    OverflowClear,
    OverflowSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExceptionKind {
    Brk,
    Cop,
}

impl ExceptionKind {
    fn vector_address(self, emulation: bool) -> u32 {
        match (self, emulation) {
            (Self::Cop, true) => 0x00FFF4,
            (Self::Brk, true) => 0x00FFFE,
            (Self::Cop, false) => 0x00FFE4,
            (Self::Brk, false) => 0x00FFE6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicroState {
    Reset {
        remaining: u8,
        low: u8,
    },
    Fetch,
    Implied(ImpliedOp),
    Immediate8(Immediate8Op),
    ImmediateLoadLow(ImmediateLoadTarget),
    ImmediateLoadHigh(ImmediateLoadTarget, u8),
    ImmediateMathLow(ImmediateMathOp),
    ImmediateMathHigh(ImmediateMathOp, u8),
    BlockMoveFirstBank(BlockMoveDirection),
    BlockMoveSecondBank(BlockMoveDirection, u8),
    BlockMoveTransfer {
        direction: BlockMoveDirection,
        source_bank: u8,
        dest_bank: u8,
    },
    BranchLongLow,
    BranchLongHigh(u8),
    Direct(DirectOp),
    DirectMathHigh {
        op: ImmediateMathOp,
        address: u16,
        low: u8,
    },
    DirectReadHigh {
        address: u16,
        low: u8,
    },
    DirectReadXHigh {
        address: u16,
        low: u8,
    },
    DirectReadYHigh {
        address: u16,
        low: u8,
    },
    DirectIncHigh {
        address: u16,
        low: u8,
    },
    DirectDecHigh {
        address: u16,
        low: u8,
    },
    DirectBitHigh {
        address: u16,
        low: u8,
    },
    DirectShiftHigh {
        op: ShiftOp,
        address: u16,
        low: u8,
    },
    DirectIndexedIndirect(DirectIndexedIndirectOp),
    DirectIndirect(DirectIndirectOp),
    DirectIndirectLong(DirectIndirectLongOp),
    DirectIndirectIndexedY(DirectIndirectIndexedYOp),
    DirectIndexedIndirectPointerHigh {
        op: DirectIndexedIndirectOp,
        pointer_addr: u16,
        low: u8,
    },
    DirectIndexedIndirectReadHigh {
        op: DirectIndexedIndirectOp,
        address: u32,
        low: u8,
    },
    StackRelative(StackRelativeOp),
    StackRelativeReadHigh {
        op: StackRelativeOp,
        address: u16,
        low: u8,
    },
    StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp),
    Stack(StackOp),
    Immediate16Low(Immediate16Op),
    Immediate16High(Immediate16Op, u8),
    PeiPointerLow,
    PeiPointerHigh {
        pointer_addr: u16,
        low: u8,
    },
    PushLow(u8),
    PullAccumulatorHigh(u8),
    PullXHigh(u8),
    PullYHigh(u8),
    PullDHigh(u8),
    Branch(BranchKind),
    AbsoluteLow(AbsoluteOp),
    AbsoluteHigh(AbsoluteOp, u8),
    AbsoluteMathHigh {
        op: ImmediateMathOp,
        address: u32,
        low: u8,
    },
    AbsoluteReadAccumulatorHigh {
        address: u32,
        low: u8,
    },
    AbsoluteReadXHigh {
        address: u32,
        low: u8,
    },
    AbsoluteReadYHigh {
        address: u32,
        low: u8,
    },
    AbsoluteBitHigh {
        address: u32,
        low: u8,
    },
    AbsoluteIncHigh {
        address: u32,
        low: u8,
    },
    AbsoluteDecHigh {
        address: u32,
        low: u8,
    },
    AbsoluteShiftHigh {
        op: ShiftOp,
        address: u32,
        low: u8,
    },
    AbsoluteLongLow(AbsoluteLongOp),
    AbsoluteLongHigh(AbsoluteLongOp, u8),
    AbsoluteLongBank(AbsoluteLongOp, u16),
    AbsoluteLongMathHigh {
        op: ImmediateMathOp,
        address: u32,
        low: u8,
    },
    AbsoluteLongReadAccumulatorHigh {
        address: u32,
        low: u8,
    },
    WriteHigh {
        address: u32,
        value: u8,
    },
    JsrPushHigh {
        target: u16,
        return_addr: u16,
    },
    JsrPushLow {
        target: u16,
        return_addr: u16,
    },
    JslPushBank {
        target_bank: u8,
        target_addr: u16,
        return_addr: u16,
    },
    JslPushHigh {
        target_bank: u8,
        target_addr: u16,
        return_addr: u16,
    },
    JslPushLow {
        target_bank: u8,
        target_addr: u16,
        return_addr: u16,
    },
    Exception(ExceptionKind),
    ExceptionPushBank {
        kind: ExceptionKind,
        return_addr: u16,
    },
    ExceptionPushHigh {
        kind: ExceptionKind,
        return_addr: u16,
    },
    ExceptionPushLow {
        kind: ExceptionKind,
        return_addr: u16,
    },
    ExceptionPushStatus(ExceptionKind),
    ExceptionVectorLow(ExceptionKind),
    ExceptionVectorHigh {
        address: u32,
        low: u8,
    },
    RtsPullLow,
    RtsPullHigh(u8),
    RtsFinalize,
    RtlPullLow,
    RtlPullHigh(u8),
    RtlPullBank(u16),
    Stopped,
}

pub(crate) struct Cpu {
    registers: Registers,
    cycles: u64,
    current_opcode: u8,
    current_state: CpuState,
    micro_state: MicroState,
    fault: Option<CpuFault>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            registers: Registers::default(),
            cycles: 0,
            current_opcode: 0,
            current_state: CpuState::Resetting,
            micro_state: MicroState::Reset {
                remaining: RESET_CYCLES,
                low: 0,
            },
            fault: None,
        }
    }
}

impl Cpu {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn registers(&self) -> &Registers {
        &self.registers
    }

    pub(crate) fn cycles(&self) -> u64 {
        self.cycles
    }

    pub(crate) fn current_opcode(&self) -> u8 {
        self.current_opcode
    }

    pub(crate) fn current_state(&self) -> CpuState {
        self.current_state
    }

    pub(crate) fn take_fault(&mut self) -> Option<CpuFault> {
        self.fault.take()
    }

    pub(crate) fn step(&mut self, bus: &mut dyn CpuBus) {
        if self.current_state == CpuState::Stopped {
            return;
        }

        self.cycles = self.cycles.wrapping_add(1);
        match self.micro_state {
            MicroState::Reset { remaining, low } => self.step_reset(bus, remaining, low),
            MicroState::Fetch => self.fetch_opcode(bus),
            MicroState::Implied(op) => self.execute_implied(op),
            MicroState::Immediate8(op) => self.execute_immediate8(bus, op),
            MicroState::ImmediateLoadLow(target) => self.execute_immediate_load_low(bus, target),
            MicroState::ImmediateLoadHigh(target, low) => {
                self.execute_immediate_load_high(bus, target, low)
            }
            MicroState::ImmediateMathLow(op) => self.execute_immediate_math_low(bus, op),
            MicroState::ImmediateMathHigh(op, low) => {
                self.execute_immediate_math_high(bus, op, low)
            }
            MicroState::BlockMoveFirstBank(direction) => {
                self.execute_block_move_first_bank(bus, direction)
            }
            MicroState::BlockMoveSecondBank(direction, first_bank) => {
                self.execute_block_move_second_bank(bus, direction, first_bank)
            }
            MicroState::BlockMoveTransfer {
                direction,
                source_bank,
                dest_bank,
            } => self.execute_block_move_transfer(bus, direction, source_bank, dest_bank),
            MicroState::BranchLongLow => self.execute_branch_long_low(bus),
            MicroState::BranchLongHigh(low) => self.execute_branch_long_high(bus, low),
            MicroState::Direct(op) => self.execute_direct(bus, op),
            MicroState::DirectMathHigh { op, address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                let value = u16::from_le_bytes([low, high]);
                self.apply_immediate_math(op, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectReadHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                self.registers.a = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.a);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectReadXHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                self.registers.x = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.x);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectReadYHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                self.registers.y = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.y);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectIncHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                let value = u16::from_le_bytes([low, high]).wrapping_add(1);
                bus.write(u32::from(address), value as u8);
                bus.write(u32::from(address.wrapping_add(1)), (value >> 8) as u8);
                self.update_nz16(value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectDecHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                let value = u16::from_le_bytes([low, high]).wrapping_sub(1);
                bus.write(u32::from(address), value as u8);
                bus.write(u32::from(address.wrapping_add(1)), (value >> 8) as u8);
                self.update_nz16(value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectBitHigh { address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                self.apply_memory_bit(u16::from_le_bytes([low, high]));
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectShiftHigh { op, address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                let value = self.apply_shift16(op, u16::from_le_bytes([low, high]));
                bus.write(u32::from(address), value as u8);
                bus.write(u32::from(address.wrapping_add(1)), (value >> 8) as u8);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::DirectIndexedIndirect(op) => self.execute_direct_indexed_indirect(bus, op),
            MicroState::DirectIndirect(op) => self.execute_direct_indirect(bus, op),
            MicroState::DirectIndirectLong(op) => self.execute_direct_indirect_long(bus, op),
            MicroState::DirectIndirectIndexedY(op) => {
                self.execute_direct_indirect_indexed_y(bus, op)
            }
            MicroState::DirectIndexedIndirectPointerHigh {
                op,
                pointer_addr,
                low,
            } => {
                let high = bus.read(u32::from(pointer_addr.wrapping_add(1)));
                let target = u16::from_le_bytes([low, high]);
                let address = self.full_data_address(target);
                let low = bus.read(address);
                if self.accumulator_is_8bit() {
                    self.apply_direct_indexed_indirect(op, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.micro_state =
                        MicroState::DirectIndexedIndirectReadHigh { op, address, low };
                }
            }
            MicroState::DirectIndexedIndirectReadHigh { op, address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = u16::from_le_bytes([low, high]);
                self.apply_direct_indexed_indirect(op, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::StackRelative(op) => self.execute_stack_relative(bus, op),
            MicroState::StackRelativeReadHigh { op, address, low } => {
                let high = bus.read(u32::from(address.wrapping_add(1)));
                let value = u16::from_le_bytes([low, high]);
                self.apply_stack_relative(op, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::StackRelativeIndirectIndexedY(op) => {
                self.execute_stack_relative_indirect_indexed_y(bus, op)
            }
            MicroState::Stack(op) => self.execute_stack(bus, op),
            MicroState::Immediate16Low(op) => self.execute_immediate16_low(bus, op),
            MicroState::Immediate16High(op, low) => self.execute_immediate16_high(bus, op, low),
            MicroState::PeiPointerLow => self.execute_pei_pointer_low(bus),
            MicroState::PeiPointerHigh { pointer_addr, low } => {
                let high = bus.read(u32::from(pointer_addr.wrapping_add(1)));
                self.stack_push(bus, high);
                self.micro_state = MicroState::PushLow(low);
            }
            MicroState::PushLow(low) => {
                self.stack_push(bus, low);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::PullAccumulatorHigh(low) => {
                let high = self.stack_pop(bus);
                self.registers.a = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.a);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::PullXHigh(low) => {
                let high = self.stack_pop(bus);
                self.registers.x = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.x);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::PullYHigh(low) => {
                let high = self.stack_pop(bus);
                self.registers.y = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.y);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::PullDHigh(low) => {
                let high = self.stack_pop(bus);
                self.registers.d = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.d);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::Branch(kind) => self.execute_branch(bus, kind),
            MicroState::AbsoluteLow(op) => self.execute_absolute_low(bus, op),
            MicroState::AbsoluteHigh(op, low) => self.execute_absolute_high(bus, op, low),
            MicroState::AbsoluteMathHigh { op, address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = u16::from_le_bytes([low, high]);
                self.apply_immediate_math(op, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteReadAccumulatorHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                self.registers.a = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.a);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteReadXHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                self.registers.x = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.x);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteReadYHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                self.registers.y = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.y);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteBitHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                self.apply_memory_bit(u16::from_le_bytes([low, high]));
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteIncHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = u16::from_le_bytes([low, high]).wrapping_add(1);
                bus.write(address, value as u8);
                bus.write(address.wrapping_add(1), (value >> 8) as u8);
                self.update_nz16(value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteDecHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = u16::from_le_bytes([low, high]).wrapping_sub(1);
                bus.write(address, value as u8);
                bus.write(address.wrapping_add(1), (value >> 8) as u8);
                self.update_nz16(value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteShiftHigh { op, address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = self.apply_shift16(op, u16::from_le_bytes([low, high]));
                bus.write(address, value as u8);
                bus.write(address.wrapping_add(1), (value >> 8) as u8);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteLongLow(op) => self.execute_absolute_long_low(bus, op),
            MicroState::AbsoluteLongHigh(op, low) => self.execute_absolute_long_high(bus, op, low),
            MicroState::AbsoluteLongBank(op, addr) => {
                self.execute_absolute_long_bank(bus, op, addr)
            }
            MicroState::AbsoluteLongMathHigh { op, address, low } => {
                let high = bus.read(address.wrapping_add(1));
                let value = u16::from_le_bytes([low, high]);
                self.apply_immediate_math(op, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::AbsoluteLongReadAccumulatorHigh { address, low } => {
                let high = bus.read(address.wrapping_add(1));
                self.registers.a = u16::from_le_bytes([low, high]);
                self.update_nz16(self.registers.a);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::WriteHigh { address, value } => {
                bus.write(address, value);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::JsrPushHigh {
                target,
                return_addr,
            } => {
                self.stack_push(bus, (return_addr >> 8) as u8);
                self.micro_state = MicroState::JsrPushLow {
                    target,
                    return_addr,
                };
            }
            MicroState::JsrPushLow {
                target,
                return_addr,
            } => {
                self.stack_push(bus, return_addr as u8);
                self.registers.pc = target;
                self.micro_state = MicroState::Fetch;
            }
            MicroState::JslPushBank {
                target_bank,
                target_addr,
                return_addr,
            } => {
                self.stack_push(bus, self.registers.pb);
                self.micro_state = MicroState::JslPushHigh {
                    target_bank,
                    target_addr,
                    return_addr,
                };
            }
            MicroState::JslPushHigh {
                target_bank,
                target_addr,
                return_addr,
            } => {
                self.stack_push(bus, (return_addr >> 8) as u8);
                self.micro_state = MicroState::JslPushLow {
                    target_bank,
                    target_addr,
                    return_addr,
                };
            }
            MicroState::JslPushLow {
                target_bank,
                target_addr,
                return_addr,
            } => {
                self.stack_push(bus, return_addr as u8);
                self.registers.pb = target_bank;
                self.registers.pc = target_addr;
                self.micro_state = MicroState::Fetch;
            }
            MicroState::Exception(kind) => self.execute_exception(kind),
            MicroState::ExceptionPushBank { kind, return_addr } => {
                self.stack_push(bus, self.registers.pb);
                self.micro_state = MicroState::ExceptionPushHigh { kind, return_addr };
            }
            MicroState::ExceptionPushHigh { kind, return_addr } => {
                self.stack_push(bus, (return_addr >> 8) as u8);
                self.micro_state = MicroState::ExceptionPushLow { kind, return_addr };
            }
            MicroState::ExceptionPushLow { kind, return_addr } => {
                self.stack_push(bus, return_addr as u8);
                self.micro_state = MicroState::ExceptionPushStatus(kind);
            }
            MicroState::ExceptionPushStatus(kind) => {
                self.stack_push(bus, self.registers.p.bits());
                self.registers.p.insert(CpuStatus::IRQ_DISABLE);
                self.registers.p.remove(CpuStatus::DECIMAL);
                self.micro_state = MicroState::ExceptionVectorLow(kind);
            }
            MicroState::ExceptionVectorLow(kind) => {
                let address = kind.vector_address(self.registers.e);
                let low = bus.read(address);
                self.micro_state = MicroState::ExceptionVectorHigh {
                    address: address.wrapping_add(1),
                    low,
                };
            }
            MicroState::ExceptionVectorHigh { address, low } => {
                let high = bus.read(address);
                self.registers.pb = 0;
                self.registers.pc = u16::from_le_bytes([low, high]);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::RtsPullLow => {
                let low = self.stack_pop(bus);
                self.micro_state = MicroState::RtsPullHigh(low);
            }
            MicroState::RtsPullHigh(low) => {
                let high = self.stack_pop(bus);
                self.registers.pc = u16::from_le_bytes([low, high]);
                self.micro_state = MicroState::RtsFinalize;
            }
            MicroState::RtsFinalize => {
                self.registers.pc = self.registers.pc.wrapping_add(1);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::RtlPullLow => {
                let low = self.stack_pop(bus);
                self.micro_state = MicroState::RtlPullHigh(low);
            }
            MicroState::RtlPullHigh(low) => {
                let high = self.stack_pop(bus);
                self.micro_state = MicroState::RtlPullBank(u16::from_le_bytes([low, high]));
            }
            MicroState::RtlPullBank(addr) => {
                let bank = self.stack_pop(bus);
                self.registers.pb = bank;
                self.registers.pc = addr.wrapping_add(1);
                self.micro_state = MicroState::Fetch;
            }
            MicroState::Stopped => {
                self.current_state = CpuState::Stopped;
            }
        }
        self.refresh_state();
    }

    fn step_reset(&mut self, bus: &mut dyn CpuBus, remaining: u8, low: u8) {
        match remaining {
            0 => self.micro_state = MicroState::Fetch,
            1 => {
                let high = bus.read(0x00FFFD);
                self.registers.pc = u16::from_le_bytes([low, high]);
                self.registers.pb = 0;
                self.registers.db = 0;
                self.registers.d = 0;
                self.micro_state = MicroState::Fetch;
            }
            2 => {
                let low = bus.read(0x00FFFC);
                self.micro_state = MicroState::Reset { remaining: 1, low };
            }
            _ => {
                self.micro_state = MicroState::Reset {
                    remaining: remaining - 1,
                    low,
                };
            }
        }
    }

    fn fetch_opcode(&mut self, bus: &mut dyn CpuBus) {
        let address = self.registers.pc;
        let opcode = bus.read(self.full_pc());
        self.current_opcode = opcode;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let state = self.decode_opcode(opcode);
        if matches!(state, MicroState::Stopped) {
            self.fault = Some(CpuFault::UnsupportedOpcode {
                opcode,
                bank: self.registers.pb,
                address,
            });
        }
        self.micro_state = state;
    }

    fn execute_implied(&mut self, op: ImpliedOp) {
        match op {
            ImpliedOp::Nop => {}
            ImpliedOp::Clc => self.registers.p.remove(CpuStatus::CARRY),
            ImpliedOp::Cld => self.registers.p.remove(CpuStatus::DECIMAL),
            ImpliedOp::Cli => self.registers.p.remove(CpuStatus::IRQ_DISABLE),
            ImpliedOp::Clv => self.registers.p.remove(CpuStatus::OVERFLOW),
            ImpliedOp::Sec => self.registers.p.insert(CpuStatus::CARRY),
            ImpliedOp::Sed => self.registers.p.insert(CpuStatus::DECIMAL),
            ImpliedOp::Sei => self.registers.p.insert(CpuStatus::IRQ_DISABLE),
            ImpliedOp::IncA => {
                if self.accumulator_is_8bit() {
                    let value = (self.registers.a as u8).wrapping_add(1);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.a = self.registers.a.wrapping_add(1);
                    self.update_nz16(self.registers.a);
                }
            }
            ImpliedOp::DecA => {
                if self.accumulator_is_8bit() {
                    let value = (self.registers.a as u8).wrapping_sub(1);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.a = self.registers.a.wrapping_sub(1);
                    self.update_nz16(self.registers.a);
                }
            }
            ImpliedOp::Inx => {
                if self.index_is_8bit() {
                    let value = (self.registers.x as u8).wrapping_add(1);
                    self.registers.x = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.x = self.registers.x.wrapping_add(1);
                    self.update_nz16(self.registers.x);
                }
            }
            ImpliedOp::Iny => {
                if self.index_is_8bit() {
                    let value = (self.registers.y as u8).wrapping_add(1);
                    self.registers.y = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.y = self.registers.y.wrapping_add(1);
                    self.update_nz16(self.registers.y);
                }
            }
            ImpliedOp::Dex => {
                if self.index_is_8bit() {
                    let value = (self.registers.x as u8).wrapping_sub(1);
                    self.registers.x = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.x = self.registers.x.wrapping_sub(1);
                    self.update_nz16(self.registers.x);
                }
            }
            ImpliedOp::Dey => {
                if self.index_is_8bit() {
                    let value = (self.registers.y as u8).wrapping_sub(1);
                    self.registers.y = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.y = self.registers.y.wrapping_sub(1);
                    self.update_nz16(self.registers.y);
                }
            }
            ImpliedOp::AslAcc => self.shift_accumulator(ShiftOp::Asl),
            ImpliedOp::RolAcc => self.shift_accumulator(ShiftOp::Rol),
            ImpliedOp::RorAcc => self.shift_accumulator(ShiftOp::Ror),
            ImpliedOp::LsrAcc => self.shift_accumulator(ShiftOp::Lsr),
            ImpliedOp::Tcd => {
                self.registers.d = self.registers.a;
                self.update_nz16(self.registers.d);
            }
            ImpliedOp::Tsc => {
                self.registers.a = self.registers.s;
                self.update_nz16(self.registers.a);
            }
            ImpliedOp::Xce => self.execute_xce(),
            ImpliedOp::Txs => {
                if self.index_is_8bit() {
                    self.registers.s = (self.registers.s & 0xFF00) | (self.registers.x & 0x00FF);
                } else {
                    self.registers.s = self.registers.x;
                }
                if self.registers.e {
                    self.registers.s = (self.registers.s & 0x00FF) | 0x0100;
                }
            }
            ImpliedOp::Stp => {
                self.micro_state = MicroState::Stopped;
                self.current_state = CpuState::Stopped;
                return;
            }
        }
        self.micro_state = MicroState::Fetch;
    }

    fn execute_direct(&mut self, bus: &mut dyn CpuBus, op: DirectOp) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let base = self.registers.d.wrapping_add(u16::from(offset));

        match op {
            DirectOp::Adc { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: base,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::AdcIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::And { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: base,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::AndIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::AndA,
                        address,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Ora { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: base,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::OraIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::OraA,
                        address,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Eor { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: base,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::EorIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::EorA,
                        address,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Inc { indexed_x, wide } => {
                let address = if indexed_x {
                    base.wrapping_add(self.registers.x)
                } else {
                    base
                };
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectIncHigh { address, low };
                } else {
                    let value = low.wrapping_add(1);
                    bus.write(u32::from(address), value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::CmpA { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: base,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::CmpAIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Bit { indexed_x, wide } => {
                let address = if indexed_x {
                    base.wrapping_add(self.registers.x)
                } else {
                    base
                };
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectBitHigh { address, low };
                } else {
                    self.apply_memory_bit(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Shift {
                op,
                indexed_x,
                wide,
            } => {
                let address = if indexed_x {
                    base.wrapping_add(self.registers.x)
                } else {
                    base
                };
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectShiftHigh { op, address, low };
                } else {
                    let value = self.apply_shift8(op, low);
                    bus.write(u32::from(address), value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Lda { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectReadHigh { address: base, low };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Sta { wide } => {
                bus.write(u32::from(base), self.registers.a as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: u32::from(base).wrapping_add(1),
                        value: (self.registers.a >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Stx { wide } => {
                bus.write(u32::from(base), self.registers.x as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: u32::from(base).wrapping_add(1),
                        value: (self.registers.x >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Dec { indexed_x, wide } => {
                let address = if indexed_x {
                    base.wrapping_add(self.registers.x)
                } else {
                    base
                };
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectDecHigh { address, low };
                } else {
                    let value = low.wrapping_sub(1);
                    bus.write(u32::from(address), value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Sty { wide } => {
                bus.write(u32::from(base), self.registers.y as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: u32::from(base).wrapping_add(1),
                        value: (self.registers.y >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::LdaIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectReadHigh { address, low };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Ldx { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectReadXHigh { address: base, low };
                } else {
                    self.load_x(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::LdxIndexedY { wide } => {
                let address = base.wrapping_add(self.registers.y);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectReadXHigh { address, low };
                } else {
                    self.load_x(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Ldy { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectReadYHigh { address: base, low };
                } else {
                    self.load_y(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::LdyIndexedX { wide } => {
                let address = base.wrapping_add(self.registers.x);
                let low = bus.read(u32::from(address));
                if wide {
                    self.micro_state = MicroState::DirectReadYHigh { address, low };
                } else {
                    self.load_y(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Cpx { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::CmpX,
                        address: base,
                        low,
                    };
                } else {
                    self.compare_x(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            DirectOp::Cpy { wide } => {
                let low = bus.read(u32::from(base));
                if wide {
                    self.micro_state = MicroState::DirectMathHigh {
                        op: ImmediateMathOp::CmpY,
                        address: base,
                        low,
                    };
                } else {
                    self.compare_y(u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
        }
    }

    fn execute_direct_indexed_indirect(
        &mut self,
        bus: &mut dyn CpuBus,
        op: DirectIndexedIndirectOp,
    ) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self
            .registers
            .d
            .wrapping_add(u16::from(offset))
            .wrapping_add(self.registers.x);
        let low = bus.read(u32::from(pointer_addr));
        self.micro_state = MicroState::DirectIndexedIndirectPointerHigh {
            op,
            pointer_addr,
            low,
        };
    }

    fn apply_direct_indexed_indirect(&mut self, op: DirectIndexedIndirectOp, value: u16) {
        match op {
            DirectIndexedIndirectOp::AdcA => {
                self.apply_immediate_math(ImmediateMathOp::AdcA, value)
            }
            DirectIndexedIndirectOp::AndA => {
                self.apply_immediate_math(ImmediateMathOp::AndA, value)
            }
            DirectIndexedIndirectOp::OraA => {
                self.apply_immediate_math(ImmediateMathOp::OraA, value)
            }
            DirectIndexedIndirectOp::EorA => {
                self.apply_immediate_math(ImmediateMathOp::EorA, value)
            }
            DirectIndexedIndirectOp::CmpA => {
                self.apply_immediate_math(ImmediateMathOp::CmpA, value)
            }
            DirectIndexedIndirectOp::Lda => self.load_accumulator(value),
        }
    }

    fn execute_direct_indirect(&mut self, bus: &mut dyn CpuBus, op: DirectIndirectOp) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self.registers.d.wrapping_add(u16::from(offset));
        let target = self.read_zero_bank_u16(bus, pointer_addr);
        let full = self.full_data_address(target);
        let value = self.read_operand_value(bus, full, !self.accumulator_is_8bit());
        self.burn_internal_cycles(
            bus,
            (if self.accumulator_is_8bit() { 3 } else { 4 }) + self.direct_page_cycle_penalty(),
        );
        self.apply_direct_indirect(op, value);
        self.micro_state = MicroState::Fetch;
    }

    fn apply_direct_indirect(&mut self, op: DirectIndirectOp, value: u16) {
        match op {
            DirectIndirectOp::AdcA => self.apply_immediate_math(ImmediateMathOp::AdcA, value),
            DirectIndirectOp::AndA => self.apply_immediate_math(ImmediateMathOp::AndA, value),
            DirectIndirectOp::OraA => self.apply_immediate_math(ImmediateMathOp::OraA, value),
            DirectIndirectOp::EorA => self.apply_immediate_math(ImmediateMathOp::EorA, value),
            DirectIndirectOp::CmpA => self.apply_immediate_math(ImmediateMathOp::CmpA, value),
            DirectIndirectOp::Lda => self.load_accumulator(value),
        }
    }

    fn execute_direct_indirect_long(&mut self, bus: &mut dyn CpuBus, op: DirectIndirectLongOp) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self.registers.d.wrapping_add(u16::from(offset));
        let full = self.read_zero_bank_u24(bus, pointer_addr);
        let value = self.read_operand_value(bus, full, !self.accumulator_is_8bit());
        self.burn_internal_cycles(
            bus,
            (if self.accumulator_is_8bit() { 4 } else { 5 }) + self.direct_page_cycle_penalty(),
        );
        self.apply_direct_indirect_long(op, value);
        self.micro_state = MicroState::Fetch;
    }

    fn apply_direct_indirect_long(&mut self, op: DirectIndirectLongOp, value: u16) {
        match op {
            DirectIndirectLongOp::AdcA => self.apply_immediate_math(ImmediateMathOp::AdcA, value),
            DirectIndirectLongOp::AndA => self.apply_immediate_math(ImmediateMathOp::AndA, value),
            DirectIndirectLongOp::OraA => self.apply_immediate_math(ImmediateMathOp::OraA, value),
            DirectIndirectLongOp::EorA => self.apply_immediate_math(ImmediateMathOp::EorA, value),
            DirectIndirectLongOp::CmpA => self.apply_immediate_math(ImmediateMathOp::CmpA, value),
            DirectIndirectLongOp::Lda => self.load_accumulator(value),
        }
    }

    fn execute_direct_indirect_indexed_y(
        &mut self,
        bus: &mut dyn CpuBus,
        op: DirectIndirectIndexedYOp,
    ) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self.registers.d.wrapping_add(u16::from(offset));
        let (full, extra_cycles) = match op {
            DirectIndirectIndexedYOp::AdcA
            | DirectIndirectIndexedYOp::AndA
            | DirectIndirectIndexedYOp::OraA
            | DirectIndirectIndexedYOp::EorA
            | DirectIndirectIndexedYOp::CmpA
            | DirectIndirectIndexedYOp::Lda => {
                let target = self.read_zero_bank_u16(bus, pointer_addr);
                (
                    self.full_data_address(target)
                        .wrapping_add(u32::from(self.registers.y)),
                    (if self.accumulator_is_8bit() { 3 } else { 4 })
                        + self.direct_page_cycle_penalty()
                        + self.direct_indirect_indexed_y_cycle_penalty(target),
                )
            }
            DirectIndirectIndexedYOp::AdcALong
            | DirectIndirectIndexedYOp::AndALong
            | DirectIndirectIndexedYOp::OraALong
            | DirectIndirectIndexedYOp::EorALong
            | DirectIndirectIndexedYOp::CmpALong
            | DirectIndirectIndexedYOp::LdaLong => (
                self.read_zero_bank_u24(bus, pointer_addr)
                    .wrapping_add(u32::from(self.registers.y)),
                (if self.accumulator_is_8bit() { 4 } else { 5 }) + self.direct_page_cycle_penalty(),
            ),
        };
        let value = self.read_operand_value(bus, full, !self.accumulator_is_8bit());
        self.burn_internal_cycles(bus, extra_cycles);
        self.apply_direct_indirect_indexed_y(op, value);
        self.micro_state = MicroState::Fetch;
    }

    fn apply_direct_indirect_indexed_y(&mut self, op: DirectIndirectIndexedYOp, value: u16) {
        match op {
            DirectIndirectIndexedYOp::AdcA
            | DirectIndirectIndexedYOp::AdcALong
            | DirectIndirectIndexedYOp::AndA
            | DirectIndirectIndexedYOp::AndALong
            | DirectIndirectIndexedYOp::OraA
            | DirectIndirectIndexedYOp::OraALong
            | DirectIndirectIndexedYOp::EorA
            | DirectIndirectIndexedYOp::EorALong
            | DirectIndirectIndexedYOp::CmpA
            | DirectIndirectIndexedYOp::CmpALong => {
                let math_op = match op {
                    DirectIndirectIndexedYOp::AdcA | DirectIndirectIndexedYOp::AdcALong => {
                        ImmediateMathOp::AdcA
                    }
                    DirectIndirectIndexedYOp::AndA | DirectIndirectIndexedYOp::AndALong => {
                        ImmediateMathOp::AndA
                    }
                    DirectIndirectIndexedYOp::OraA | DirectIndirectIndexedYOp::OraALong => {
                        ImmediateMathOp::OraA
                    }
                    DirectIndirectIndexedYOp::EorA | DirectIndirectIndexedYOp::EorALong => {
                        ImmediateMathOp::EorA
                    }
                    DirectIndirectIndexedYOp::CmpA | DirectIndirectIndexedYOp::CmpALong => {
                        ImmediateMathOp::CmpA
                    }
                    DirectIndirectIndexedYOp::Lda | DirectIndirectIndexedYOp::LdaLong => {
                        unreachable!("LDA handled below")
                    }
                };
                self.apply_immediate_math(math_op, value)
            }
            DirectIndirectIndexedYOp::Lda | DirectIndirectIndexedYOp::LdaLong => {
                self.load_accumulator(value)
            }
        }
    }

    fn apply_stack_relative(&mut self, op: StackRelativeOp, value: u16) {
        match op {
            StackRelativeOp::AdcA => self.apply_immediate_math(ImmediateMathOp::AdcA, value),
            StackRelativeOp::AndA => self.apply_immediate_math(ImmediateMathOp::AndA, value),
            StackRelativeOp::OraA => self.apply_immediate_math(ImmediateMathOp::OraA, value),
            StackRelativeOp::EorA => self.apply_immediate_math(ImmediateMathOp::EorA, value),
            StackRelativeOp::CmpA => self.apply_immediate_math(ImmediateMathOp::CmpA, value),
            StackRelativeOp::Lda => self.load_accumulator(value),
        }
    }

    fn apply_stack_relative_indirect_indexed_y(
        &mut self,
        op: StackRelativeIndirectIndexedYOp,
        value: u16,
    ) {
        match op {
            StackRelativeIndirectIndexedYOp::AdcA => {
                self.apply_immediate_math(ImmediateMathOp::AdcA, value)
            }
            StackRelativeIndirectIndexedYOp::AndA => {
                self.apply_immediate_math(ImmediateMathOp::AndA, value)
            }
            StackRelativeIndirectIndexedYOp::OraA => {
                self.apply_immediate_math(ImmediateMathOp::OraA, value)
            }
            StackRelativeIndirectIndexedYOp::EorA => {
                self.apply_immediate_math(ImmediateMathOp::EorA, value)
            }
            StackRelativeIndirectIndexedYOp::CmpA => {
                self.apply_immediate_math(ImmediateMathOp::CmpA, value)
            }
            StackRelativeIndirectIndexedYOp::Lda => self.load_accumulator(value),
        }
    }

    fn execute_stack_relative(&mut self, bus: &mut dyn CpuBus, op: StackRelativeOp) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let address = self.registers.s.wrapping_add(u16::from(offset));
        let low = bus.read(u32::from(address));
        if self.accumulator_is_8bit() {
            self.apply_stack_relative(op, u16::from(low));
            self.micro_state = MicroState::Fetch;
        } else {
            self.micro_state = MicroState::StackRelativeReadHigh { op, address, low };
        }
    }

    fn execute_stack_relative_indirect_indexed_y(
        &mut self,
        bus: &mut dyn CpuBus,
        op: StackRelativeIndirectIndexedYOp,
    ) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self.registers.s.wrapping_add(u16::from(offset));
        let target = self.read_zero_bank_u16(bus, pointer_addr);
        let full = self
            .full_data_address(target)
            .wrapping_add(u32::from(self.registers.y));
        let value = self.read_operand_value(bus, full, !self.accumulator_is_8bit());
        self.burn_internal_cycles(bus, if self.accumulator_is_8bit() { 5 } else { 6 });
        self.apply_stack_relative_indirect_indexed_y(op, value);
        self.micro_state = MicroState::Fetch;
    }

    fn execute_branch(&mut self, bus: &mut dyn CpuBus, kind: BranchKind) {
        let offset = bus.read(self.full_pc()) as i8;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        if self.branch_taken(kind) {
            self.registers.pc = self.registers.pc.wrapping_add_signed(i16::from(offset));
        }
        self.micro_state = MicroState::Fetch;
    }

    fn execute_exception(&mut self, kind: ExceptionKind) {
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let return_addr = self.registers.pc;
        if self.registers.e {
            self.micro_state = MicroState::ExceptionPushHigh { kind, return_addr };
        } else {
            self.micro_state = MicroState::ExceptionPushBank { kind, return_addr };
        }
    }

    fn execute_branch_long_low(&mut self, bus: &mut dyn CpuBus) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::BranchLongHigh(low);
    }

    fn execute_branch_long_high(&mut self, bus: &mut dyn CpuBus, low: u8) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let offset = i16::from_le_bytes([low, high]);
        self.registers.pc = self.registers.pc.wrapping_add_signed(offset);
        self.micro_state = MicroState::Fetch;
    }

    fn execute_immediate8(&mut self, bus: &mut dyn CpuBus, op: Immediate8Op) {
        let value = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        match op {
            Immediate8Op::Rep => self.apply_status_mask(value, false),
            Immediate8Op::Sep => self.apply_status_mask(value, true),
            Immediate8Op::Wdm => {}
        }
        self.micro_state = MicroState::Fetch;
    }

    fn execute_immediate16_low(&mut self, bus: &mut dyn CpuBus, op: Immediate16Op) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::Immediate16High(op, low);
    }

    fn execute_immediate16_high(&mut self, bus: &mut dyn CpuBus, op: Immediate16Op, low: u8) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let value = match op {
            Immediate16Op::Pea => u16::from_le_bytes([low, high]),
            Immediate16Op::Per => {
                let offset = i16::from_le_bytes([low, high]);
                self.registers.pc.wrapping_add_signed(offset)
            }
        };
        self.stack_push(bus, (value >> 8) as u8);
        self.micro_state = MicroState::PushLow(value as u8);
    }

    fn execute_pei_pointer_low(&mut self, bus: &mut dyn CpuBus) {
        let offset = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let pointer_addr = self.registers.d.wrapping_add(u16::from(offset));
        let low = bus.read(u32::from(pointer_addr));
        self.micro_state = MicroState::PeiPointerHigh { pointer_addr, low };
    }

    fn execute_immediate_load_low(&mut self, bus: &mut dyn CpuBus, target: ImmediateLoadTarget) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        if self.load_width(target) == 1 {
            self.apply_immediate_load(target, u16::from(low));
            self.micro_state = MicroState::Fetch;
        } else {
            self.micro_state = MicroState::ImmediateLoadHigh(target, low);
        }
    }

    fn execute_immediate_load_high(
        &mut self,
        bus: &mut dyn CpuBus,
        target: ImmediateLoadTarget,
        low: u8,
    ) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.apply_immediate_load(target, u16::from_le_bytes([low, high]));
        self.micro_state = MicroState::Fetch;
    }

    fn execute_immediate_math_low(&mut self, bus: &mut dyn CpuBus, op: ImmediateMathOp) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        if self.immediate_math_width(op) == 1 {
            self.apply_immediate_math(op, u16::from(low));
            self.micro_state = MicroState::Fetch;
        } else {
            self.micro_state = MicroState::ImmediateMathHigh(op, low);
        }
    }

    fn execute_immediate_math_high(&mut self, bus: &mut dyn CpuBus, op: ImmediateMathOp, low: u8) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.apply_immediate_math(op, u16::from_le_bytes([low, high]));
        self.micro_state = MicroState::Fetch;
    }

    fn execute_block_move_first_bank(
        &mut self,
        bus: &mut dyn CpuBus,
        direction: BlockMoveDirection,
    ) {
        let first_bank = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::BlockMoveSecondBank(direction, first_bank);
    }

    fn execute_block_move_second_bank(
        &mut self,
        bus: &mut dyn CpuBus,
        direction: BlockMoveDirection,
        dest_bank: u8,
    ) {
        let source_bank = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::BlockMoveTransfer {
            direction,
            source_bank,
            dest_bank,
        };
    }

    fn execute_block_move_transfer(
        &mut self,
        bus: &mut dyn CpuBus,
        direction: BlockMoveDirection,
        source_bank: u8,
        dest_bank: u8,
    ) {
        let source = ((source_bank as u32) << 16) | u32::from(self.registers.x);
        let dest = ((dest_bank as u32) << 16) | u32::from(self.registers.y);
        let value = bus.read(source);
        bus.write(dest, value);
        self.registers.db = dest_bank;
        self.registers.a = self.registers.a.wrapping_sub(1);
        self.adjust_block_move_indexes(direction);
        self.burn_internal_cycles(bus, 6);
        self.micro_state = if self.registers.a == 0xFFFF {
            MicroState::Fetch
        } else {
            MicroState::BlockMoveTransfer {
                direction,
                source_bank,
                dest_bank,
            }
        };
    }

    fn execute_stack(&mut self, bus: &mut dyn CpuBus, op: StackOp) {
        match op {
            StackOp::Pha => {
                if self.accumulator_is_8bit() {
                    self.stack_push(bus, self.registers.a as u8);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.stack_push(bus, (self.registers.a >> 8) as u8);
                    self.micro_state = MicroState::PushLow(self.registers.a as u8);
                }
            }
            StackOp::Pla => {
                let low = self.stack_pop(bus);
                if self.accumulator_is_8bit() {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.micro_state = MicroState::PullAccumulatorHigh(low);
                }
            }
            StackOp::Php => {
                self.stack_push(bus, self.registers.p.bits());
                self.micro_state = MicroState::Fetch;
            }
            StackOp::Plp => {
                let value = self.stack_pop(bus);
                self.set_status(value);
                self.micro_state = MicroState::Fetch;
            }
            StackOp::Phx => {
                if self.index_is_8bit() {
                    self.stack_push(bus, self.registers.x as u8);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.stack_push(bus, (self.registers.x >> 8) as u8);
                    self.micro_state = MicroState::PushLow(self.registers.x as u8);
                }
            }
            StackOp::Phy => {
                if self.index_is_8bit() {
                    self.stack_push(bus, self.registers.y as u8);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.stack_push(bus, (self.registers.y >> 8) as u8);
                    self.micro_state = MicroState::PushLow(self.registers.y as u8);
                }
            }
            StackOp::Phb => {
                self.stack_push(bus, self.registers.db);
                self.micro_state = MicroState::Fetch;
            }
            StackOp::Phk => {
                self.stack_push(bus, self.registers.pb);
                self.micro_state = MicroState::Fetch;
            }
            StackOp::Phd => {
                self.stack_push(bus, (self.registers.d >> 8) as u8);
                self.micro_state = MicroState::PushLow(self.registers.d as u8);
            }
            StackOp::Plx => {
                let low = self.stack_pop(bus);
                if self.index_is_8bit() {
                    self.registers.x = u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.micro_state = MicroState::PullXHigh(low);
                }
            }
            StackOp::Ply => {
                let low = self.stack_pop(bus);
                if self.index_is_8bit() {
                    self.registers.y = u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                } else {
                    self.micro_state = MicroState::PullYHigh(low);
                }
            }
            StackOp::Plb => {
                let value = self.stack_pop(bus);
                self.registers.db = value;
                self.update_nz8(value);
                self.micro_state = MicroState::Fetch;
            }
            StackOp::Pld => {
                let low = self.stack_pop(bus);
                self.micro_state = MicroState::PullDHigh(low);
            }
        }
    }

    fn execute_absolute_low(&mut self, bus: &mut dyn CpuBus, op: AbsoluteOp) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::AbsoluteHigh(op, low);
    }

    fn execute_absolute_high(&mut self, bus: &mut dyn CpuBus, op: AbsoluteOp, low: u8) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let address = u16::from_le_bytes([low, high]);

        match op {
            AbsoluteOp::Adc { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::AdcIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::AdcIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::And { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::AndIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::AndIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Ora { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::OraIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::OraIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Eor { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::EorIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::EorIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Inc { indexed_x, wide } => {
                let full = if indexed_x {
                    self.full_data_address(address)
                        .wrapping_add(u32::from(self.registers.x))
                } else {
                    self.full_data_address(address)
                };
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteIncHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    let value = value.wrapping_add(1);
                    bus.write(full, value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::CmpA { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::CmpAIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::CmpAIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Dec { indexed_x, wide } => {
                let full = if indexed_x {
                    self.full_data_address(address)
                        .wrapping_add(u32::from(self.registers.x))
                } else {
                    self.full_data_address(address)
                };
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteDecHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    let value = value.wrapping_sub(1);
                    bus.write(full, value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Cpx { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::CmpX,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpX, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Cpy { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteMathHigh {
                        op: ImmediateMathOp::CmpY,
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpY, u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Ldx { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadXHigh {
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.load_x(u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::LdxIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadXHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    self.load_x(u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Ldy { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadYHigh {
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.load_y(u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::LdyIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadYHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    self.load_y(u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Lda { wide } => {
                let value = self.read_data_bank(bus, address);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadAccumulatorHigh {
                        address: self.full_data_address(address),
                        low: value,
                    };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::LdaIndexedX { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.x));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadAccumulatorHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::LdaIndexedY { wide } => {
                let full = self
                    .full_data_address(address)
                    .wrapping_add(u32::from(self.registers.y));
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteReadAccumulatorHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Sta { wide } => {
                self.write_bus(bus, address, self.registers.a as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: self.full_data_address(address).wrapping_add(1),
                        value: (self.registers.a >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Sty { wide } => {
                self.write_bus(bus, address, self.registers.y as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: self.full_data_address(address).wrapping_add(1),
                        value: (self.registers.y >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Stz { wide } => {
                self.write_bus(bus, address, 0);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: self.full_data_address(address).wrapping_add(1),
                        value: 0,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Bit { indexed_x, wide } => {
                let full = if indexed_x {
                    self.full_data_address(address)
                        .wrapping_add(u32::from(self.registers.x))
                } else {
                    self.full_data_address(address)
                };
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteBitHigh {
                        address: full,
                        low: value,
                    };
                } else {
                    self.apply_memory_bit(u16::from(value));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Shift {
                op,
                indexed_x,
                wide,
            } => {
                let full = if indexed_x {
                    self.full_data_address(address)
                        .wrapping_add(u32::from(self.registers.x))
                } else {
                    self.full_data_address(address)
                };
                let value = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteShiftHigh {
                        op,
                        address: full,
                        low: value,
                    };
                } else {
                    let value = self.apply_shift8(op, value);
                    bus.write(full, value);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Jmp => {
                self.registers.pc = address;
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteOp::JmpIndirect => {
                self.registers.pc = self.read_zero_bank_u16(bus, address);
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteOp::JmpIndexedXIndirect => {
                let pointer = address.wrapping_add(self.registers.x);
                self.registers.pc = self.read_program_bank_u16(bus, pointer);
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteOp::JmlIndirect => {
                let full = self.read_zero_bank_u24(bus, address);
                self.registers.pb = (full >> 16) as u8;
                self.registers.pc = full as u16;
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteOp::Jsr => {
                let return_addr = self.registers.pc.wrapping_sub(1);
                self.micro_state = MicroState::JsrPushHigh {
                    target: address,
                    return_addr,
                };
            }
            AbsoluteOp::JsrIndexedXIndirect => {
                let return_addr = self.registers.pc.wrapping_sub(1);
                let target =
                    self.read_program_bank_u16(bus, address.wrapping_add(self.registers.x));
                self.micro_state = MicroState::JsrPushHigh {
                    target,
                    return_addr,
                };
            }
        }
    }

    fn execute_absolute_long_low(&mut self, bus: &mut dyn CpuBus, op: AbsoluteLongOp) {
        let low = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::AbsoluteLongHigh(op, low);
    }

    fn execute_absolute_long_high(&mut self, bus: &mut dyn CpuBus, op: AbsoluteLongOp, low: u8) {
        let high = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.micro_state = MicroState::AbsoluteLongBank(op, u16::from_le_bytes([low, high]));
    }

    fn execute_absolute_long_bank(
        &mut self,
        bus: &mut dyn CpuBus,
        op: AbsoluteLongOp,
        address: u16,
    ) {
        let bank = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        let full = ((bank as u32) << 16) | u32::from(address);

        match op {
            AbsoluteLongOp::Adc { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: full,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::AdcIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::AdcA,
                        address: indexed,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AdcA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::And { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: full,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::AndIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::AndA,
                        address: indexed,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::AndA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::Ora { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: full,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::OraIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::OraA,
                        address: indexed,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::OraA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::Eor { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: full,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::EorIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::EorA,
                        address: indexed,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::EorA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::CmpA { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: full,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::CmpAIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongMathHigh {
                        op: ImmediateMathOp::CmpA,
                        address: indexed,
                        low,
                    };
                } else {
                    self.apply_immediate_math(ImmediateMathOp::CmpA, u16::from(low));
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::Lda { wide } => {
                let low = bus.read(full);
                if wide {
                    self.micro_state =
                        MicroState::AbsoluteLongReadAccumulatorHigh { address: full, low };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::LdaIndexedX { wide } => {
                let indexed = full.wrapping_add(u32::from(self.registers.x));
                let low = bus.read(indexed);
                if wide {
                    self.micro_state = MicroState::AbsoluteLongReadAccumulatorHigh {
                        address: indexed,
                        low,
                    };
                } else {
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(low);
                    self.update_nz8(low);
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::Sta { wide } => {
                bus.write(full, self.registers.a as u8);
                if wide {
                    self.micro_state = MicroState::WriteHigh {
                        address: full.wrapping_add(1),
                        value: (self.registers.a >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteLongOp::Jml => {
                self.registers.pb = bank;
                self.registers.pc = address;
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteLongOp::Jsl => {
                let return_addr = self.registers.pc.wrapping_sub(1);
                self.micro_state = MicroState::JslPushBank {
                    target_bank: bank,
                    target_addr: address,
                    return_addr,
                };
            }
        }
    }

    fn execute_xce(&mut self) {
        let carry = self.registers.p.contains(CpuStatus::CARRY);
        let old_emulation = self.registers.e;

        if old_emulation {
            self.registers.p.insert(CpuStatus::CARRY);
        } else {
            self.registers.p.remove(CpuStatus::CARRY);
        }

        self.registers.e = carry;
        if self.registers.e {
            self.registers
                .p
                .insert(CpuStatus::ACCUMULATOR_8BIT | CpuStatus::INDEX_8BIT);
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
            self.registers.s = (self.registers.s & 0x00FF) | 0x0100;
        }
    }

    fn apply_status_mask(&mut self, value: u8, set: bool) {
        let mask = CpuStatus::from_bits_truncate(value);
        if set {
            self.registers.p.insert(mask);
        } else {
            self.registers.p.remove(mask);
        }

        self.normalize_status_after_mode_change();
    }

    fn set_status(&mut self, value: u8) {
        self.registers.p = CpuStatus::from_bits_truncate(value);
        self.normalize_status_after_mode_change();
    }

    fn normalize_status_after_mode_change(&mut self) {
        if self.registers.e {
            self.registers
                .p
                .insert(CpuStatus::ACCUMULATOR_8BIT | CpuStatus::INDEX_8BIT);
        }

        if self.index_is_8bit() {
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
        }
    }

    fn apply_immediate_load(&mut self, target: ImmediateLoadTarget, value: u16) {
        match target {
            ImmediateLoadTarget::A => {
                if self.accumulator_is_8bit() {
                    let value = value as u8;
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.a = value;
                    self.update_nz16(value);
                }
            }
            ImmediateLoadTarget::X => {
                if self.index_is_8bit() {
                    let value = value as u8;
                    self.registers.x = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.x = value;
                    self.update_nz16(value);
                }
            }
            ImmediateLoadTarget::Y => {
                if self.index_is_8bit() {
                    let value = value as u8;
                    self.registers.y = u16::from(value);
                    self.update_nz8(value);
                } else {
                    self.registers.y = value;
                    self.update_nz16(value);
                }
            }
        }
    }

    fn load_accumulator(&mut self, value: u16) {
        if self.accumulator_is_8bit() {
            let value = value as u8;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz8(value);
        } else {
            self.registers.a = value;
            self.update_nz16(value);
        }
    }

    fn load_x(&mut self, value: u16) {
        if self.index_is_8bit() {
            let value = value as u8;
            self.registers.x = u16::from(value);
            self.update_nz8(value);
        } else {
            self.registers.x = value;
            self.update_nz16(value);
        }
    }

    fn load_y(&mut self, value: u16) {
        if self.index_is_8bit() {
            let value = value as u8;
            self.registers.y = u16::from(value);
            self.update_nz8(value);
        } else {
            self.registers.y = value;
            self.update_nz16(value);
        }
    }

    fn apply_immediate_math(&mut self, op: ImmediateMathOp, value: u16) {
        match op {
            ImmediateMathOp::BitA => {
                if self.accumulator_is_8bit() {
                    self.registers.p.set(
                        CpuStatus::ZERO,
                        (self.registers.a as u8) & (value as u8) == 0,
                    );
                } else {
                    self.registers
                        .p
                        .set(CpuStatus::ZERO, self.registers.a & value == 0);
                }
            }
            ImmediateMathOp::AndA => {
                if self.accumulator_is_8bit() {
                    let result = (self.registers.a as u8) & (value as u8);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
                    self.update_nz8(result);
                } else {
                    self.registers.a &= value;
                    self.update_nz16(self.registers.a);
                }
            }
            ImmediateMathOp::OraA => {
                if self.accumulator_is_8bit() {
                    let result = (self.registers.a as u8) | (value as u8);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
                    self.update_nz8(result);
                } else {
                    self.registers.a |= value;
                    self.update_nz16(self.registers.a);
                }
            }
            ImmediateMathOp::EorA => {
                if self.accumulator_is_8bit() {
                    let result = (self.registers.a as u8) ^ (value as u8);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
                    self.update_nz8(result);
                } else {
                    self.registers.a ^= value;
                    self.update_nz16(self.registers.a);
                }
            }
            ImmediateMathOp::AdcA => {
                if self.accumulator_is_8bit() {
                    let a = self.registers.a as u8;
                    let operand = value as u8;
                    let carry_in = u8::from(self.registers.p.contains(CpuStatus::CARRY));
                    let (sum1, carry1) = a.overflowing_add(operand);
                    let (result, carry2) = sum1.overflowing_add(carry_in);
                    self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
                    self.registers.p.set(CpuStatus::CARRY, carry1 || carry2);
                    self.registers.p.set(
                        CpuStatus::OVERFLOW,
                        ((!(a ^ operand) & (a ^ result)) & 0x80) != 0,
                    );
                    self.update_nz8(result);
                } else {
                    let a = self.registers.a;
                    let carry_in = u16::from(self.registers.p.contains(CpuStatus::CARRY));
                    let (sum1, carry1) = a.overflowing_add(value);
                    let (result, carry2) = sum1.overflowing_add(carry_in);
                    self.registers.a = result;
                    self.registers.p.set(CpuStatus::CARRY, carry1 || carry2);
                    self.registers.p.set(
                        CpuStatus::OVERFLOW,
                        ((!(a ^ value) & (a ^ result)) & 0x8000) != 0,
                    );
                    self.update_nz16(result);
                }
            }
            ImmediateMathOp::CmpA => {
                if self.accumulator_is_8bit() {
                    self.compare_value8(self.registers.a as u8, value as u8);
                } else {
                    self.compare_value16(self.registers.a, value);
                }
            }
            ImmediateMathOp::CmpX => self.compare_x(value),
            ImmediateMathOp::CmpY => self.compare_y(value),
        }
    }

    fn immediate_math_width(&self, op: ImmediateMathOp) -> u8 {
        match op {
            ImmediateMathOp::BitA
            | ImmediateMathOp::AndA
            | ImmediateMathOp::OraA
            | ImmediateMathOp::EorA
            | ImmediateMathOp::AdcA
            | ImmediateMathOp::CmpA
                if self.accumulator_is_8bit() =>
            {
                1
            }
            ImmediateMathOp::CmpX | ImmediateMathOp::CmpY if self.index_is_8bit() => 1,
            _ => 2,
        }
    }

    fn decode_opcode(&mut self, opcode: u8) -> MicroState {
        match opcode {
            0x00 => MicroState::Exception(ExceptionKind::Brk),
            0x02 => MicroState::Exception(ExceptionKind::Cop),
            0xEA => MicroState::Implied(ImpliedOp::Nop),
            0x18 => MicroState::Implied(ImpliedOp::Clc),
            0xD8 => MicroState::Implied(ImpliedOp::Cld),
            0x58 => MicroState::Implied(ImpliedOp::Cli),
            0xB8 => MicroState::Implied(ImpliedOp::Clv),
            0x38 => MicroState::Implied(ImpliedOp::Sec),
            0xF8 => MicroState::Implied(ImpliedOp::Sed),
            0x78 => MicroState::Implied(ImpliedOp::Sei),
            0x0A => MicroState::Implied(ImpliedOp::AslAcc),
            0x2A => MicroState::Implied(ImpliedOp::RolAcc),
            0x6A => MicroState::Implied(ImpliedOp::RorAcc),
            0x1A => MicroState::Implied(ImpliedOp::IncA),
            0x3A => MicroState::Implied(ImpliedOp::DecA),
            0xE8 => MicroState::Implied(ImpliedOp::Inx),
            0xC8 => MicroState::Implied(ImpliedOp::Iny),
            0xCA => MicroState::Implied(ImpliedOp::Dex),
            0x88 => MicroState::Implied(ImpliedOp::Dey),
            0x4A => MicroState::Implied(ImpliedOp::LsrAcc),
            0x5B => MicroState::Implied(ImpliedOp::Tcd),
            0x3B => MicroState::Implied(ImpliedOp::Tsc),
            0xFB => MicroState::Implied(ImpliedOp::Xce),
            0x9A => MicroState::Implied(ImpliedOp::Txs),
            0xDB => MicroState::Implied(ImpliedOp::Stp),
            0x80 => MicroState::Branch(BranchKind::Always),
            0x90 => MicroState::Branch(BranchKind::CarryClear),
            0xB0 => MicroState::Branch(BranchKind::CarrySet),
            0xF0 => MicroState::Branch(BranchKind::Equal),
            0xD0 => MicroState::Branch(BranchKind::NotEqual),
            0x30 => MicroState::Branch(BranchKind::Minus),
            0x10 => MicroState::Branch(BranchKind::Plus),
            0x50 => MicroState::Branch(BranchKind::OverflowClear),
            0x70 => MicroState::Branch(BranchKind::OverflowSet),
            0xC2 => MicroState::Immediate8(Immediate8Op::Rep),
            0xE2 => MicroState::Immediate8(Immediate8Op::Sep),
            0x42 => MicroState::Immediate8(Immediate8Op::Wdm),
            0x44 => MicroState::BlockMoveFirstBank(BlockMoveDirection::Decrement),
            0x54 => MicroState::BlockMoveFirstBank(BlockMoveDirection::Increment),
            0x62 => MicroState::Immediate16Low(Immediate16Op::Per),
            0x82 => MicroState::BranchLongLow,
            0xA9 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::A),
            0xA2 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::X),
            0xA0 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::Y),
            0x29 => MicroState::ImmediateMathLow(ImmediateMathOp::AndA),
            0x09 => MicroState::ImmediateMathLow(ImmediateMathOp::OraA),
            0x49 => MicroState::ImmediateMathLow(ImmediateMathOp::EorA),
            0x69 => MicroState::ImmediateMathLow(ImmediateMathOp::AdcA),
            0x89 => MicroState::ImmediateMathLow(ImmediateMathOp::BitA),
            0xC9 => MicroState::ImmediateMathLow(ImmediateMathOp::CmpA),
            0xE0 => MicroState::ImmediateMathLow(ImmediateMathOp::CmpX),
            0xC0 => MicroState::ImmediateMathLow(ImmediateMathOp::CmpY),
            0x85 => MicroState::Direct(DirectOp::Sta {
                wide: !self.accumulator_is_8bit(),
            }),
            0x06 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Asl,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x26 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Rol,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x46 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Lsr,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x66 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Ror,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x86 => MicroState::Direct(DirectOp::Stx {
                wide: !self.index_is_8bit(),
            }),
            0x23 => MicroState::StackRelative(StackRelativeOp::AndA),
            0x03 => MicroState::StackRelative(StackRelativeOp::OraA),
            0x43 => MicroState::StackRelative(StackRelativeOp::EorA),
            0x25 => MicroState::Direct(DirectOp::And {
                wide: !self.accumulator_is_8bit(),
            }),
            0x05 => MicroState::Direct(DirectOp::Ora {
                wide: !self.accumulator_is_8bit(),
            }),
            0x45 => MicroState::Direct(DirectOp::Eor {
                wide: !self.accumulator_is_8bit(),
            }),
            0xA4 => MicroState::Direct(DirectOp::Ldy {
                wide: !self.index_is_8bit(),
            }),
            0xC4 => MicroState::Direct(DirectOp::Cpy {
                wide: !self.index_is_8bit(),
            }),
            0xC5 => MicroState::Direct(DirectOp::CmpA {
                wide: !self.accumulator_is_8bit(),
            }),
            0xE6 => MicroState::Direct(DirectOp::Inc {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x24 => MicroState::Direct(DirectOp::Bit {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x84 => MicroState::Direct(DirectOp::Sty {
                wide: !self.index_is_8bit(),
            }),
            0xA5 => MicroState::Direct(DirectOp::Lda {
                wide: !self.accumulator_is_8bit(),
            }),
            0x65 => MicroState::Direct(DirectOp::Adc {
                wide: !self.accumulator_is_8bit(),
            }),
            0x35 => MicroState::Direct(DirectOp::AndIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x15 => MicroState::Direct(DirectOp::OraIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x55 => MicroState::Direct(DirectOp::EorIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xD5 => MicroState::Direct(DirectOp::CmpAIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xF6 => MicroState::Direct(DirectOp::Inc {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x34 => MicroState::Direct(DirectOp::Bit {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0xD6 => MicroState::Direct(DirectOp::Dec {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x16 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Asl,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x36 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Rol,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x56 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Lsr,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x76 => MicroState::Direct(DirectOp::Shift {
                op: ShiftOp::Ror,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x75 => MicroState::Direct(DirectOp::AdcIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xB4 => MicroState::Direct(DirectOp::LdyIndexedX {
                wide: !self.index_is_8bit(),
            }),
            0xA6 => MicroState::Direct(DirectOp::Ldx {
                wide: !self.index_is_8bit(),
            }),
            0xB6 => MicroState::Direct(DirectOp::LdxIndexedY {
                wide: !self.index_is_8bit(),
            }),
            0xB5 => MicroState::Direct(DirectOp::LdaIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xC6 => MicroState::Direct(DirectOp::Dec {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0xE4 => MicroState::Direct(DirectOp::Cpx {
                wide: !self.index_is_8bit(),
            }),
            0x61 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::AdcA),
            0x21 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::AndA),
            0x01 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::OraA),
            0x41 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::EorA),
            0xA1 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::Lda),
            0xC1 => MicroState::DirectIndexedIndirect(DirectIndexedIndirectOp::CmpA),
            0x63 => MicroState::StackRelative(StackRelativeOp::AdcA),
            0xA3 => MicroState::StackRelative(StackRelativeOp::Lda),
            0xC3 => MicroState::StackRelative(StackRelativeOp::CmpA),
            0x67 => MicroState::DirectIndirectLong(DirectIndirectLongOp::AdcA),
            0x27 => MicroState::DirectIndirectLong(DirectIndirectLongOp::AndA),
            0x07 => MicroState::DirectIndirectLong(DirectIndirectLongOp::OraA),
            0x47 => MicroState::DirectIndirectLong(DirectIndirectLongOp::EorA),
            0xA7 => MicroState::DirectIndirectLong(DirectIndirectLongOp::Lda),
            0xC7 => MicroState::DirectIndirectLong(DirectIndirectLongOp::CmpA),
            0x31 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::AndA),
            0x11 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::OraA),
            0x51 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::EorA),
            0xB1 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::Lda),
            0x32 => MicroState::DirectIndirect(DirectIndirectOp::AndA),
            0x12 => MicroState::DirectIndirect(DirectIndirectOp::OraA),
            0x52 => MicroState::DirectIndirect(DirectIndirectOp::EorA),
            0xB2 => MicroState::DirectIndirect(DirectIndirectOp::Lda),
            0xD1 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::CmpA),
            0xD2 => MicroState::DirectIndirect(DirectIndirectOp::CmpA),
            0xD4 => MicroState::PeiPointerLow,
            0x33 => {
                MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::AndA)
            }
            0x13 => {
                MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::OraA)
            }
            0x53 => {
                MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::EorA)
            }
            0xB3 => MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::Lda),
            0xD3 => {
                MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::CmpA)
            }
            0x37 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::AndALong),
            0x17 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::OraALong),
            0x57 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::EorALong),
            0xB7 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::LdaLong),
            0x71 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::AdcA),
            0x72 => MicroState::DirectIndirect(DirectIndirectOp::AdcA),
            0x73 => {
                MicroState::StackRelativeIndirectIndexedY(StackRelativeIndirectIndexedYOp::AdcA)
            }
            0x77 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::AdcALong),
            0xD7 => MicroState::DirectIndirectIndexedY(DirectIndirectIndexedYOp::CmpALong),
            0x39 => MicroState::AbsoluteLow(AbsoluteOp::AndIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0x19 => MicroState::AbsoluteLow(AbsoluteOp::OraIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0x59 => MicroState::AbsoluteLow(AbsoluteOp::EorIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0xB9 => MicroState::AbsoluteLow(AbsoluteOp::LdaIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0xD9 => MicroState::AbsoluteLow(AbsoluteOp::CmpAIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0x79 => MicroState::AbsoluteLow(AbsoluteOp::AdcIndexedY {
                wide: !self.accumulator_is_8bit(),
            }),
            0x2D => MicroState::AbsoluteLow(AbsoluteOp::And {
                wide: !self.accumulator_is_8bit(),
            }),
            0x0D => MicroState::AbsoluteLow(AbsoluteOp::Ora {
                wide: !self.accumulator_is_8bit(),
            }),
            0x4D => MicroState::AbsoluteLow(AbsoluteOp::Eor {
                wide: !self.accumulator_is_8bit(),
            }),
            0xCD => MicroState::AbsoluteLow(AbsoluteOp::CmpA {
                wide: !self.accumulator_is_8bit(),
            }),
            0xAC => MicroState::AbsoluteLow(AbsoluteOp::Ldy {
                wide: !self.index_is_8bit(),
            }),
            0xCC => MicroState::AbsoluteLow(AbsoluteOp::Cpy {
                wide: !self.index_is_8bit(),
            }),
            0xCE => MicroState::AbsoluteLow(AbsoluteOp::Dec {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0xEC => MicroState::AbsoluteLow(AbsoluteOp::Cpx {
                wide: !self.index_is_8bit(),
            }),
            0xEE => MicroState::AbsoluteLow(AbsoluteOp::Inc {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x6D => MicroState::AbsoluteLow(AbsoluteOp::Adc {
                wide: !self.accumulator_is_8bit(),
            }),
            0x3D => MicroState::AbsoluteLow(AbsoluteOp::AndIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x1D => MicroState::AbsoluteLow(AbsoluteOp::OraIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x5D => MicroState::AbsoluteLow(AbsoluteOp::EorIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xBC => MicroState::AbsoluteLow(AbsoluteOp::LdyIndexedX {
                wide: !self.index_is_8bit(),
            }),
            0xBD => MicroState::AbsoluteLow(AbsoluteOp::LdaIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xBE => MicroState::AbsoluteLow(AbsoluteOp::LdxIndexedY {
                wide: !self.index_is_8bit(),
            }),
            0xDD => MicroState::AbsoluteLow(AbsoluteOp::CmpAIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xFE => MicroState::AbsoluteLow(AbsoluteOp::Inc {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0xDE => MicroState::AbsoluteLow(AbsoluteOp::Dec {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x7D => MicroState::AbsoluteLow(AbsoluteOp::AdcIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xAD => MicroState::AbsoluteLow(AbsoluteOp::Lda {
                wide: !self.accumulator_is_8bit(),
            }),
            0xAE => MicroState::AbsoluteLow(AbsoluteOp::Ldx {
                wide: !self.index_is_8bit(),
            }),
            0x8D => MicroState::AbsoluteLow(AbsoluteOp::Sta {
                wide: !self.accumulator_is_8bit(),
            }),
            0x8C => MicroState::AbsoluteLow(AbsoluteOp::Sty {
                wide: !self.index_is_8bit(),
            }),
            0x9C => MicroState::AbsoluteLow(AbsoluteOp::Stz {
                wide: !self.accumulator_is_8bit(),
            }),
            0x2C => MicroState::AbsoluteLow(AbsoluteOp::Bit {
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x3C => MicroState::AbsoluteLow(AbsoluteOp::Bit {
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x0E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Asl,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x2E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Rol,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x4E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Lsr,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x6E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Ror,
                indexed_x: false,
                wide: !self.accumulator_is_8bit(),
            }),
            0x6C => MicroState::AbsoluteLow(AbsoluteOp::JmpIndirect),
            0x1E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Asl,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x3E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Rol,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x5E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Lsr,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x7E => MicroState::AbsoluteLow(AbsoluteOp::Shift {
                op: ShiftOp::Ror,
                indexed_x: true,
                wide: !self.accumulator_is_8bit(),
            }),
            0x7C => MicroState::AbsoluteLow(AbsoluteOp::JmpIndexedXIndirect),
            0x4C => MicroState::AbsoluteLow(AbsoluteOp::Jmp),
            0x20 => MicroState::AbsoluteLow(AbsoluteOp::Jsr),
            0xFC => MicroState::AbsoluteLow(AbsoluteOp::JsrIndexedXIndirect),
            0xDC => MicroState::AbsoluteLow(AbsoluteOp::JmlIndirect),
            0x2F => MicroState::AbsoluteLongLow(AbsoluteLongOp::And {
                wide: !self.accumulator_is_8bit(),
            }),
            0x0F => MicroState::AbsoluteLongLow(AbsoluteLongOp::Ora {
                wide: !self.accumulator_is_8bit(),
            }),
            0x4F => MicroState::AbsoluteLongLow(AbsoluteLongOp::Eor {
                wide: !self.accumulator_is_8bit(),
            }),
            0xCF => MicroState::AbsoluteLongLow(AbsoluteLongOp::CmpA {
                wide: !self.accumulator_is_8bit(),
            }),
            0x6F => MicroState::AbsoluteLongLow(AbsoluteLongOp::Adc {
                wide: !self.accumulator_is_8bit(),
            }),
            0x3F => MicroState::AbsoluteLongLow(AbsoluteLongOp::AndIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x1F => MicroState::AbsoluteLongLow(AbsoluteLongOp::OraIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x5F => MicroState::AbsoluteLongLow(AbsoluteLongOp::EorIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xDF => MicroState::AbsoluteLongLow(AbsoluteLongOp::CmpAIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x7F => MicroState::AbsoluteLongLow(AbsoluteLongOp::AdcIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0xAF => MicroState::AbsoluteLongLow(AbsoluteLongOp::Lda {
                wide: !self.accumulator_is_8bit(),
            }),
            0xBF => MicroState::AbsoluteLongLow(AbsoluteLongOp::LdaIndexedX {
                wide: !self.accumulator_is_8bit(),
            }),
            0x22 => MicroState::AbsoluteLongLow(AbsoluteLongOp::Jsl),
            0x5C => MicroState::AbsoluteLongLow(AbsoluteLongOp::Jml),
            0x8F => MicroState::AbsoluteLongLow(AbsoluteLongOp::Sta {
                wide: !self.accumulator_is_8bit(),
            }),
            0x48 => MicroState::Stack(StackOp::Pha),
            0x68 => MicroState::Stack(StackOp::Pla),
            0x08 => MicroState::Stack(StackOp::Php),
            0x28 => MicroState::Stack(StackOp::Plp),
            0xDA => MicroState::Stack(StackOp::Phx),
            0x5A => MicroState::Stack(StackOp::Phy),
            0x8B => MicroState::Stack(StackOp::Phb),
            0x4B => MicroState::Stack(StackOp::Phk),
            0x0B => MicroState::Stack(StackOp::Phd),
            0xFA => MicroState::Stack(StackOp::Plx),
            0x7A => MicroState::Stack(StackOp::Ply),
            0xAB => MicroState::Stack(StackOp::Plb),
            0x2B => MicroState::Stack(StackOp::Pld),
            0xF4 => MicroState::Immediate16Low(Immediate16Op::Pea),
            0x60 => MicroState::RtsPullLow,
            0x6B => MicroState::RtlPullLow,
            _ => MicroState::Stopped,
        }
    }

    fn full_pc(&self) -> u32 {
        ((self.registers.pb as u32) << 16) | (self.registers.pc as u32)
    }

    fn write_bus(&mut self, bus: &mut dyn CpuBus, address: u16, value: u8) {
        bus.write(self.full_data_address(address), value);
    }

    fn read_data_bank(&mut self, bus: &mut dyn CpuBus, address: u16) -> u8 {
        bus.read(self.full_data_address(address))
    }

    fn full_data_address(&self, address: u16) -> u32 {
        ((self.registers.db as u32) << 16) | u32::from(address)
    }

    fn read_zero_bank_u16(&mut self, bus: &mut dyn CpuBus, address: u16) -> u16 {
        let low = bus.read(u32::from(address));
        let high = bus.read(u32::from(address.wrapping_add(1)));
        u16::from_le_bytes([low, high])
    }

    fn read_zero_bank_u24(&mut self, bus: &mut dyn CpuBus, address: u16) -> u32 {
        let target = self.read_zero_bank_u16(bus, address);
        let bank = bus.read(u32::from(address.wrapping_add(2)));
        ((bank as u32) << 16) | u32::from(target)
    }

    fn read_program_bank_u16(&mut self, bus: &mut dyn CpuBus, address: u16) -> u16 {
        let bank_base = (self.registers.pb as u32) << 16;
        let low = bus.read(bank_base | u32::from(address));
        let high = bus.read(bank_base | u32::from(address.wrapping_add(1)));
        u16::from_le_bytes([low, high])
    }

    fn read_operand_value(&mut self, bus: &mut dyn CpuBus, address: u32, wide: bool) -> u16 {
        let low = bus.read(address);
        if wide {
            let high = bus.read(address.wrapping_add(1));
            u16::from_le_bytes([low, high])
        } else {
            u16::from(low)
        }
    }

    fn adjust_block_move_indexes(&mut self, direction: BlockMoveDirection) {
        let adjust = |value: u16, index_8bit: bool, direction| match (index_8bit, direction) {
            (true, BlockMoveDirection::Increment) => u16::from((value as u8).wrapping_add(1)),
            (true, BlockMoveDirection::Decrement) => u16::from((value as u8).wrapping_sub(1)),
            (false, BlockMoveDirection::Increment) => value.wrapping_add(1),
            (false, BlockMoveDirection::Decrement) => value.wrapping_sub(1),
        };
        self.registers.x = adjust(self.registers.x, self.index_is_8bit(), direction);
        self.registers.y = adjust(self.registers.y, self.index_is_8bit(), direction);
    }

    fn burn_internal_cycles(&mut self, bus: &mut dyn CpuBus, additional: u8) {
        for _ in 0..additional {
            self.cycles = self.cycles.wrapping_add(1);
            bus.tick();
        }
    }

    fn direct_page_cycle_penalty(&self) -> u8 {
        u8::from((self.registers.d & 0x00FF) != 0)
    }

    fn direct_indirect_indexed_y_cycle_penalty(&self, base: u16) -> u8 {
        if !self.index_is_8bit() {
            1
        } else {
            u8::from(u16::from(base as u8) + (self.registers.y & 0x00FF) > 0x00FF)
        }
    }

    fn stack_push(&mut self, bus: &mut dyn CpuBus, value: u8) {
        bus.write(u32::from(self.registers.s), value);
        self.registers.s = self.registers.s.wrapping_sub(1);
        if self.registers.e {
            self.registers.s = (self.registers.s & 0x00FF) | 0x0100;
        }
    }

    fn stack_pop(&mut self, bus: &mut dyn CpuBus) -> u8 {
        self.registers.s = self.registers.s.wrapping_add(1);
        if self.registers.e {
            self.registers.s = (self.registers.s & 0x00FF) | 0x0100;
        }
        bus.read(u32::from(self.registers.s))
    }

    fn accumulator_is_8bit(&self) -> bool {
        self.registers.e || self.registers.p.contains(CpuStatus::ACCUMULATOR_8BIT)
    }

    fn index_is_8bit(&self) -> bool {
        self.registers.e || self.registers.p.contains(CpuStatus::INDEX_8BIT)
    }

    fn load_width(&self, target: ImmediateLoadTarget) -> u8 {
        match target {
            ImmediateLoadTarget::A if self.accumulator_is_8bit() => 1,
            ImmediateLoadTarget::X | ImmediateLoadTarget::Y if self.index_is_8bit() => 1,
            _ => 2,
        }
    }

    fn shift_accumulator(&mut self, op: ShiftOp) {
        if self.accumulator_is_8bit() {
            let value = self.apply_shift8(op, self.registers.a as u8);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
        } else {
            self.registers.a = self.apply_shift16(op, self.registers.a);
        }
    }

    fn apply_shift8(&mut self, op: ShiftOp, value: u8) -> u8 {
        match op {
            ShiftOp::Asl => {
                self.registers.p.set(CpuStatus::CARRY, value & 0x80 != 0);
                let result = value << 1;
                self.update_nz8(result);
                result
            }
            ShiftOp::Rol => {
                let carry_in = u8::from(self.registers.p.contains(CpuStatus::CARRY));
                self.registers.p.set(CpuStatus::CARRY, value & 0x80 != 0);
                let result = (value << 1) | carry_in;
                self.update_nz8(result);
                result
            }
            ShiftOp::Ror => {
                let carry_in = u8::from(self.registers.p.contains(CpuStatus::CARRY)) << 7;
                self.registers.p.set(CpuStatus::CARRY, value & 0x01 != 0);
                let result = (value >> 1) | carry_in;
                self.update_nz8(result);
                result
            }
            ShiftOp::Lsr => {
                self.registers.p.set(CpuStatus::CARRY, value & 0x01 != 0);
                let result = value >> 1;
                self.update_nz8(result);
                result
            }
        }
    }

    fn apply_shift16(&mut self, op: ShiftOp, value: u16) -> u16 {
        match op {
            ShiftOp::Asl => {
                self.registers.p.set(CpuStatus::CARRY, value & 0x8000 != 0);
                let result = value << 1;
                self.update_nz16(result);
                result
            }
            ShiftOp::Rol => {
                let carry_in = u16::from(self.registers.p.contains(CpuStatus::CARRY));
                self.registers.p.set(CpuStatus::CARRY, value & 0x8000 != 0);
                let result = (value << 1) | carry_in;
                self.update_nz16(result);
                result
            }
            ShiftOp::Ror => {
                let carry_in = u16::from(self.registers.p.contains(CpuStatus::CARRY)) << 15;
                self.registers.p.set(CpuStatus::CARRY, value & 0x0001 != 0);
                let result = (value >> 1) | carry_in;
                self.update_nz16(result);
                result
            }
            ShiftOp::Lsr => {
                self.registers.p.set(CpuStatus::CARRY, value & 0x0001 != 0);
                let result = value >> 1;
                self.update_nz16(result);
                result
            }
        }
    }

    fn apply_memory_bit(&mut self, value: u16) {
        if self.accumulator_is_8bit() {
            let value = value as u8;
            self.registers
                .p
                .set(CpuStatus::ZERO, (self.registers.a as u8) & value == 0);
            self.registers.p.set(CpuStatus::NEGATIVE, value & 0x80 != 0);
            self.registers.p.set(CpuStatus::OVERFLOW, value & 0x40 != 0);
        } else {
            self.registers
                .p
                .set(CpuStatus::ZERO, self.registers.a & value == 0);
            self.registers
                .p
                .set(CpuStatus::NEGATIVE, value & 0x8000 != 0);
            self.registers
                .p
                .set(CpuStatus::OVERFLOW, value & 0x4000 != 0);
        }
    }

    fn branch_taken(&self, kind: BranchKind) -> bool {
        match kind {
            BranchKind::Always => true,
            BranchKind::CarryClear => !self.registers.p.contains(CpuStatus::CARRY),
            BranchKind::CarrySet => self.registers.p.contains(CpuStatus::CARRY),
            BranchKind::Equal => self.registers.p.contains(CpuStatus::ZERO),
            BranchKind::NotEqual => !self.registers.p.contains(CpuStatus::ZERO),
            BranchKind::Minus => self.registers.p.contains(CpuStatus::NEGATIVE),
            BranchKind::Plus => !self.registers.p.contains(CpuStatus::NEGATIVE),
            BranchKind::OverflowClear => !self.registers.p.contains(CpuStatus::OVERFLOW),
            BranchKind::OverflowSet => self.registers.p.contains(CpuStatus::OVERFLOW),
        }
    }

    fn update_nz8(&mut self, value: u8) {
        self.registers.p.set(CpuStatus::ZERO, value == 0);
        self.registers.p.set(CpuStatus::NEGATIVE, value & 0x80 != 0);
    }

    fn update_nz16(&mut self, value: u16) {
        self.registers.p.set(CpuStatus::ZERO, value == 0);
        self.registers
            .p
            .set(CpuStatus::NEGATIVE, value & 0x8000 != 0);
    }

    fn compare_x(&mut self, value: u16) {
        if self.index_is_8bit() {
            self.compare_value8(self.registers.x as u8, value as u8);
        } else {
            self.compare_value16(self.registers.x, value);
        }
    }

    fn compare_y(&mut self, value: u16) {
        if self.index_is_8bit() {
            self.compare_value8(self.registers.y as u8, value as u8);
        } else {
            self.compare_value16(self.registers.y, value);
        }
    }

    fn compare_value8(&mut self, left: u8, right: u8) {
        let result = left.wrapping_sub(right);
        self.registers.p.set(CpuStatus::CARRY, left >= right);
        self.update_nz8(result);
    }

    fn compare_value16(&mut self, left: u16, right: u16) {
        let result = left.wrapping_sub(right);
        self.registers.p.set(CpuStatus::CARRY, left >= right);
        self.update_nz16(result);
    }

    fn refresh_state(&mut self) {
        self.current_state = match self.micro_state {
            MicroState::Reset { .. } => CpuState::Resetting,
            MicroState::Stopped => CpuState::Stopped,
            _ => CpuState::Running,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{Cpu, CpuState, CpuStatus};
    use crate::bus::CpuBus;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct TestBus {
        memory: BTreeMap<u32, u8>,
    }

    impl TestBus {
        fn with_reset_vector(vector: u16) -> Self {
            let mut system = Self::default();
            system.load(0x00FFFC, &vector.to_le_bytes());
            system
        }

        fn load_native_exception_vectors(&mut self, cop: u16, brk: u16) {
            self.load(0x00FFE4, &cop.to_le_bytes());
            self.load(0x00FFE6, &brk.to_le_bytes());
        }

        fn load(&mut self, base: u32, bytes: &[u8]) {
            for (offset, value) in bytes.iter().copied().enumerate() {
                self.memory.insert(base + offset as u32, value);
            }
        }
    }

    impl CpuBus for TestBus {
        fn read(&mut self, addr: u32) -> u8 {
            *self.memory.get(&addr).unwrap_or(&0)
        }

        fn write(&mut self, addr: u32, data: u8) {
            self.memory.insert(addr, data);
        }
    }

    fn step_n(cpu: &mut Cpu, system: &mut TestBus, cycles: usize) {
        for _ in 0..cycles {
            cpu.step(system);
        }
    }

    fn run_until_stopped(cpu: &mut Cpu, system: &mut TestBus, max_cycles: usize) {
        for _ in 0..max_cycles {
            cpu.step(system);
            if cpu.current_state() == CpuState::Stopped {
                return;
            }
        }

        panic!("CPU did not stop within {max_cycles} cycles");
    }

    #[test]
    fn reset_takes_seven_cycles_and_populates_register_snapshot() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x1234);

        step_n(&mut cpu, &mut system, 7);

        assert_eq!(cpu.current_state(), CpuState::Running);
        assert_eq!(cpu.cycles(), 7);
        assert_eq!(cpu.registers().pc(), 0x1234);
        assert_eq!(cpu.registers().pb(), 0x00);
        assert_eq!(cpu.registers().db(), 0x00);
        assert_eq!(cpu.registers().d(), 0x0000);
        assert_eq!(cpu.registers().s(), 0x01FF);
        assert_eq!(
            cpu.registers().status(),
            CpuStatus::IRQ_DISABLE | CpuStatus::INDEX_8BIT | CpuStatus::ACCUMULATOR_8BIT
        );
        assert!(cpu.registers().emulation_mode());
    }

    #[test]
    fn wrapper_tracks_fetched_opcode_and_stopped_state() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0xEA, 0xDB]);

        step_n(&mut cpu, &mut system, 8);
        assert_eq!(cpu.current_opcode(), 0xEA);
        assert_eq!(cpu.registers().pc(), 0x8001);
        assert_eq!(cpu.current_state(), CpuState::Running);

        run_until_stopped(&mut cpu, &mut system, 8);
        assert_eq!(cpu.current_opcode(), 0xDB);
        assert_eq!(cpu.current_state(), CpuState::Stopped);
    }

    #[test]
    fn wrapper_exposes_native_mode_bootstrap_registers() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 32);

        assert!(!cpu.registers().emulation_mode());
        assert_eq!(cpu.registers().x(), 0x01EF);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert!(
            !cpu.registers()
                .status()
                .contains(CpuStatus::ACCUMULATOR_8BIT | CpuStatus::INDEX_8BIT)
        );
    }

    #[test]
    fn absolute_store_instructions_write_expected_bytes() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0xA9, 0x34, 0x8D, 0x34, 0x12, 0x9C, 0x35, 0x12, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 32);

        assert_eq!(system.memory.get(&0x001234), Some(&0x34));
        assert_eq!(system.memory.get(&0x001235), Some(&0x00));
    }

    #[test]
    fn absolute_load_instruction_reads_expected_bytes() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x001234, &[0xED]);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xAD, 0x34, 0x12, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().a(), 0x00ED);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn jsr_and_rts_restore_program_counter_and_stack() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x20, 0x06, 0x80, 0xDB, 0xEA, 0xEA, 0x18, 0x60]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.current_state(), CpuState::Stopped);
        assert_eq!(cpu.registers().pc(), 0x8004);
        assert_eq!(cpu.registers().s(), 0x01FF);
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
    }

    #[test]
    fn direct_page_indexed_loads_and_branches_work() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x01, 0x85, 0x10, 0xC6, 0x10, 0xF0, 0x01,
                0xDB, 0xA2, 0x10, 0x00, 0xB5, 0x00, 0x10, 0x01, 0xDB, 0xE8, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(system.memory.get(&0x0010), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0x0000);
        assert_eq!(cpu.registers().x(), 0x0011);
    }

    #[test]
    fn bcs_bvs_and_bvc_branch_when_condition_matches() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA0, 0x00, 0x00, 0x38, 0xB0, 0x01, 0xDB, 0xC8, 0xE2, 0x40,
                0x70, 0x01, 0xDB, 0xC8, 0xC2, 0x40, 0x50, 0x01, 0xDB, 0xC8, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().y(), 0x0003);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::OVERFLOW));
    }

    #[test]
    fn bcs_bvs_and_bvc_fall_through_when_condition_mismatches() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0x00, 0x00, 0x18, 0xB0, 0x01, 0xE8, 0xC2, 0x40, 0x70,
                0x01, 0xE8, 0xE2, 0x40, 0x50, 0x01, 0xE8, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().x(), 0x0003);
        assert!(cpu.registers().status().contains(CpuStatus::OVERFLOW));
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
    }

    #[test]
    fn brk_native_mode_pushes_bank_pc_and_status_then_vectors_to_bank_zero() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load_native_exception_vectors(0x9004, 0x9000);
        system.load(0x009000, &[0xDB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x34, 0x12, 0xA2, 0x56, 0x34, 0xA0, 0x78, 0x56, 0xC2,
                0xF4, 0xE2, 0x0B, 0x5C, 0x00, 0x80, 0x7E,
            ],
        );
        system.load(0x7E8000, &[0x00, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 192);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().x(), 0x3456);
        assert_eq!(cpu.registers().y(), 0x5678);
        assert_eq!(cpu.registers().pb(), 0x00);
        assert_eq!(cpu.registers().pc(), 0x9001);
        assert_eq!(cpu.registers().s(), 0x01FB);
        assert_eq!(cpu.registers().status().bits() & 0xFF, 0x07);
        assert_eq!(system.memory.get(&0x0001FF), Some(&0x7E));
        assert_eq!(system.memory.get(&0x0001FE), Some(&0x80));
        assert_eq!(system.memory.get(&0x0001FD), Some(&0x02));
        assert_eq!(system.memory.get(&0x0001FC), Some(&0x0B));
    }

    #[test]
    fn cop_native_mode_uses_cop_vector_and_pushes_return_state() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load_native_exception_vectors(0x9004, 0x9000);
        system.load(0x009004, &[0xDB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x34, 0x12, 0xA2, 0x56, 0x34, 0xA0, 0x78, 0x56, 0xC2,
                0xF4, 0xE2, 0x0B, 0x5C, 0x00, 0x80, 0x7E,
            ],
        );
        system.load(0x7E8000, &[0x02, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 192);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().x(), 0x3456);
        assert_eq!(cpu.registers().y(), 0x5678);
        assert_eq!(cpu.registers().pb(), 0x00);
        assert_eq!(cpu.registers().pc(), 0x9005);
        assert_eq!(cpu.registers().s(), 0x01FB);
        assert_eq!(cpu.registers().status().bits() & 0xFF, 0x07);
        assert_eq!(system.memory.get(&0x0001FF), Some(&0x7E));
        assert_eq!(system.memory.get(&0x0001FE), Some(&0x80));
        assert_eq!(system.memory.get(&0x0001FD), Some(&0x02));
        assert_eq!(system.memory.get(&0x0001FC), Some(&0x0B));
    }

    #[test]
    fn brl_applies_a_signed_16bit_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x82, 0x03, 0x00, 0xDB, 0xDB, 0xDB, 0xA9, 0x7F, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x007F);
    }

    #[test]
    fn cld_cli_clv_and_sed_update_only_their_target_flags() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xE2, 0xFF, 0xD8, 0x58, 0xB8, 0xF8, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().status().bits() & 0xFF, 0xBB);
    }

    #[test]
    fn absolute_bit_and_sty_support_wait_loop_primitives() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA0, 0x21, 0x00, 0x8C, 0x16, 0x21, 0xA9, 0x80,
                0x8D, 0x00, 0x20, 0x2C, 0x00, 0x20, 0x30, 0x01, 0xDB, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(system.memory.get(&0x002116), Some(&0x21));
        assert_eq!(system.memory.get(&0x002117), Some(&0x00));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn jsl_rtl_and_long_addressing_restore_caller_context() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x34, 0x12, 0x22, 0x10, 0x80, 0x01, 0xDB, 0xEA, 0xEA,
                0xEA, 0xEA,
            ],
        );
        system.load(
            0x018010,
            &[
                0x8F, 0x00, 0x00, 0x7E, 0xA9, 0x78, 0x56, 0x5C, 0x1B, 0x80, 0x01, 0x8F, 0x02, 0x00,
                0x7E, 0x6B,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x7E0000), Some(&0x34));
        assert_eq!(system.memory.get(&0x7E0001), Some(&0x12));
        assert_eq!(system.memory.get(&0x7E0002), Some(&0x78));
        assert_eq!(system.memory.get(&0x7E0003), Some(&0x56));
        assert_eq!(cpu.registers().pb(), 0x00);
        assert_eq!(cpu.registers().pc(), 0x800C);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn jmp_indirect_reads_zero_bank_target() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFF0, &[0x10, 0x80]);
        system.load(0x008000, &[0x6C, 0xF0, 0xFF, 0xDB]);
        system.load(0x008010, &[0xA9, 0x42, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x0042);
        assert_eq!(cpu.registers().pb(), 0x00);
        assert_eq!(cpu.registers().pc(), 0x8013);
    }

    #[test]
    fn jml_indirect_reads_zero_bank_long_target() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFF2, &[0x00, 0x80, 0x7E]);
        system.load(0x008000, &[0xDC, 0xF2, 0xFF, 0xDB]);
        system.load(0x7E8000, &[0xA9, 0x5A, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x005A);
        assert_eq!(cpu.registers().pb(), 0x7E);
        assert_eq!(cpu.registers().pc(), 0x8003);
    }

    #[test]
    fn jmp_indexed_x_indirect_uses_program_bank_and_wraps() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x5C, 0x00, 0x70, 0x7E]);
        system.load(0x7E7000, &[0xA2, 0x81, 0x7C, 0xFF, 0xFF, 0xDB]);
        system.load(0x7E0080, &[0x00, 0x80]);
        system.load(0x7E8000, &[0xA9, 0x99, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x0099);
        assert_eq!(cpu.registers().x(), 0x0081);
        assert_eq!(cpu.registers().pb(), 0x7E);
        assert_eq!(cpu.registers().pc(), 0x8003);
    }

    #[test]
    fn jsr_indexed_x_indirect_uses_program_bank_and_wraps() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x5C, 0x00, 0x70, 0x7E]);
        system.load(0x7E7000, &[0xA2, 0x81, 0xFC, 0xFF, 0xFF, 0xDB]);
        system.load(0x7E0080, &[0x00, 0x80]);
        system.load(0x7E8000, &[0xA9, 0x5A, 0x60]);

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x005A);
        assert_eq!(cpu.registers().x(), 0x0081);
        assert_eq!(cpu.registers().pb(), 0x7E);
        assert_eq!(cpu.registers().pc(), 0x7006);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn mvn_copies_forward_across_bank_wrap() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x21]);
        system.load(0x7E0000, &[0x22, 0x23, 0x24]);
        system.load(0x7F0001, &[0x00, 0x99]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x03, 0x00, 0xA2, 0xFF, 0xFF, 0xA0, 0xFE, 0xFF, 0x54,
                0x7F, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 256);

        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().x(), 0x0003);
        assert_eq!(cpu.registers().y(), 0x0002);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(system.memory.get(&0x7FFFFE), Some(&0x21));
        assert_eq!(system.memory.get(&0x7FFFFF), Some(&0x22));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x23));
        assert_eq!(system.memory.get(&0x7F0001), Some(&0x24));
        assert_eq!(system.memory.get(&0x7F0002), Some(&0x99));
    }

    #[test]
    fn mvn_with_8bit_indexes_wraps_low_bytes() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7E00FF, &[0x51]);
        system.load(0x7E0000, &[0x52, 0x53, 0x54]);
        system.load(0x7F0001, &[0x00, 0x99]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x03, 0x00, 0xA2, 0xFF, 0x05, 0xA0, 0xFE, 0x05, 0xE2,
                0x10, 0x54, 0x7F, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 256);

        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().x(), 0x0003);
        assert_eq!(cpu.registers().y(), 0x0002);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(system.memory.get(&0x7F00FE), Some(&0x51));
        assert_eq!(system.memory.get(&0x7F00FF), Some(&0x52));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x53));
        assert_eq!(system.memory.get(&0x7F0001), Some(&0x54));
        assert_eq!(system.memory.get(&0x7F0002), Some(&0x99));
    }

    #[test]
    fn mvp_copies_backward_across_bank_wrap() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x21]);
        system.load(0x7E0000, &[0x22, 0x23, 0x24]);
        system.load(0x7F0001, &[0x00]);
        system.load(0x7FFFFD, &[0x99]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x03, 0x00, 0xA2, 0x02, 0x00, 0xA0, 0x01, 0x00, 0x44,
                0x7F, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 256);

        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().x(), 0xFFFE);
        assert_eq!(cpu.registers().y(), 0xFFFD);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(system.memory.get(&0x7FFFFE), Some(&0x21));
        assert_eq!(system.memory.get(&0x7FFFFF), Some(&0x22));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x23));
        assert_eq!(system.memory.get(&0x7F0001), Some(&0x24));
        assert_eq!(system.memory.get(&0x7FFFFD), Some(&0x99));
    }

    #[test]
    fn mvp_with_8bit_indexes_wraps_low_bytes() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7E00FF, &[0x51]);
        system.load(0x7E0000, &[0x52, 0x53, 0x54]);
        system.load(0x7F0001, &[0x00, 0x99]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x03, 0x00, 0xA2, 0x02, 0x05, 0xA0, 0x01, 0x05, 0xE2,
                0x10, 0x44, 0x7F, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 256);

        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().x(), 0x00FE);
        assert_eq!(cpu.registers().y(), 0x00FD);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(system.memory.get(&0x7F00FE), Some(&0x51));
        assert_eq!(system.memory.get(&0x7F00FF), Some(&0x52));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x53));
        assert_eq!(system.memory.get(&0x7F0001), Some(&0x54));
        assert_eq!(system.memory.get(&0x7F0002), Some(&0x99));
    }

    #[test]
    fn stack_immediate_math_and_compare_primitives_match_bootstrap_needs() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x01, 0x85, 0x10, 0xA2, 0x02, 0x00, 0xCA,
                0xE4, 0x10, 0xD0, 0x0E, 0xA9, 0xB5, 0x48, 0x4A, 0x68, 0x29, 0x0F, 0x69, 0x03, 0xC9,
                0x09, 0x90, 0x01, 0xC8, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x0009);
        assert_eq!(cpu.registers().x(), 0x0001);
        assert_eq!(cpu.registers().y(), 0x0001);
        assert_eq!(cpu.registers().s(), 0x01FF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn bit_immediate_16bit_updates_only_zero_flag() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x77, 0x93, 0xE2, 0xC0, 0x89, 0x34, 0x12, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x9377);
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(cpu.registers().status().contains(CpuStatus::OVERFLOW));
    }

    #[test]
    fn bit_immediate_8bit_preserves_nv_and_sets_zero() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x55, 0x00, 0xE2, 0xE0, 0x89, 0xAA, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x0055);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(cpu.registers().status().contains(CpuStatus::OVERFLOW));
    }

    #[test]
    fn cpx_and_cpy_immediate_support_16bit_test_checks() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0x56, 0x34, 0xA0, 0x78, 0x56, 0xE0, 0x56, 0x34, 0xD0,
                0x02, 0xC0, 0x78, 0x56, 0xD0, 0x01, 0xC8, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().x(), 0x3456);
        assert_eq!(cpu.registers().y(), 0x5679);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_direct_indexed_indirect_uses_direct_page_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x7F, 0x48, 0xAB, 0xC2, 0x20, 0xA9, 0xFF,
                0xFF, 0x5B, 0xA9, 0xCD, 0xAB, 0xA2, 0x91, 0xFF, 0xC2, 0xFF, 0xC1, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().x(), 0xFF91);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_stack_relative_reads_from_stack_window() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000201, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0xCD, 0xAB, 0xC2, 0xFF, 0xC3,
                0x12, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_stack_relative_indirect_indexed_y_uses_stack_pointer_and_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x0001FF, &[0xDC, 0xFE]);
        system.load(0x7F0FEC, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48,
                0xAB, 0xC2, 0x20, 0xA9, 0xCD, 0xAB, 0xA0, 0x10, 0x11, 0xC2, 0xFF, 0xD3, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 176);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert_eq!(cpu.registers().y(), 0x1110);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0xCD, 0xAB, 0xA2, 0x33, 0x01,
                0xC2, 0xFF, 0xD5, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xCD]);
        system.load(0x7F0300, &[0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xCD, 0xAB, 0xA0, 0x00, 0x03, 0xC2, 0xFF, 0xD9, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cmp_absolute_long_indexed_x_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xCD]);
        system.load(0x7F0300, &[0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xCD, 0xAB, 0xA2, 0x00, 0x03, 0xC2, 0xFF, 0xDF, 0xFF,
                0xFF, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().a(), 0xABCD);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cpx_absolute_reads_from_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA2, 0xCD, 0xAB, 0xC2, 0xFF, 0xEC, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().x(), 0xABCD);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cpy_direct_reads_wrapped_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA0, 0xCD, 0xAB,
                0xC2, 0xFF, 0xC4, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().y(), 0xABCD);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn cpy_absolute_reads_from_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0xCD, 0xAB]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA0, 0xCD, 0xAB, 0xC2, 0xFF, 0xCC, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().y(), 0xABCD);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dec_accumulator_16bit_updates_zero() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x01, 0x00, 0x3A, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dec_accumulator_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x01, 0x12, 0xE2, 0x20, 0x3A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x1200);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dec_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x01, 0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0xD6, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x00));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dec_absolute_indexed_x_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x01]);
        system.load(0x7F0300, &[0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0xA2,
                0x00, 0x03, 0xDE, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7F02FF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0x00));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn inc_direct_16bit_wraps_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xFF, 0xFF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xE6, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000033), Some(&0x00));
        assert_eq!(system.memory.get(&0x000034), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn inc_absolute_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0xFF]);
        system.load(0x7F0000, &[0xFF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xEE, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7EFFFF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn inc_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0xFF, 0xFF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA2, 0x33, 0x01,
                0xF6, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x00));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn inc_absolute_indexed_x_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xFF]);
        system.load(0x7F0300, &[0xFF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA2, 0x00, 0x03, 0xFE, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7F02FF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dey_16bit_updates_flags() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x10, 0xA0, 0x00, 0x80, 0x88, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().y(), 0x7FFF);
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn dey_8bit_wraps_with_zero_extended_register() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x18, 0xFB, 0xA0, 0x00, 0x88, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().y(), 0x00FF);
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_indexed_indirect_uses_direct_page_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x7F, 0x48, 0xAB, 0xC2, 0x20, 0xA9, 0xFF,
                0xFF, 0x5B, 0xA9, 0x12, 0x11, 0xA2, 0x91, 0xFF, 0xE2, 0x20, 0x38, 0x61, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_stack_relative_reads_from_stack_window() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000201, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x12, 0x11, 0xE2, 0x20, 0x38,
                0x63, 0x12, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_reads_from_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x12, 0x11, 0xE2, 0x20, 0x38, 0x65, 0x33, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn lda_direct_reads_from_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xED]);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA5, 0x33, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x00ED);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_immediate_16bit_updates_negative_flag() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x00, 0x80, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldx_and_ldy_immediate_8bit_update_zero_flag() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0xA2, 0x00, 0xA0, 0x00, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().x(), 0x0000);
        assert_eq!(cpu.registers().y(), 0x0000);
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn lda_direct_indexed_indirect_uses_direct_page_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA2, 0x91, 0xFF, 0xA1, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().x(), 0xFF91);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_stack_relative_reads_from_stack_window() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000201, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x34, 0x12, 0xA3, 0x12, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_direct_indirect_long_reads_full_24bit_pointer() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x34, 0x12, 0x7F]);
        system.load(0x7F1234, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA7, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_direct_indirect_reads_via_data_bank_pointer() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x34, 0x12]);
        system.load(0x7F1234, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xB2, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_direct_indirect_indexed_y_applies_y_offset_in_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA0, 0x00, 0x11, 0xB1, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_stack_relative_indirect_indexed_y_uses_stack_pointer_and_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x0001FF, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48,
                0xAB, 0xC2, 0x20, 0xA9, 0x34, 0x12, 0xA0, 0x00, 0x11, 0xB3, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_direct_indirect_long_indexed_y_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE, 0x7E]);
        system.load(0x7F0FDC, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA0, 0x00, 0x11,
                0xB7, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA0, 0x00, 0x03, 0xB9, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lda_absolute_indexed_x_and_long_indexed_x_carry_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA2, 0x00, 0x03, 0xBD, 0xFF, 0xFF, 0xBF, 0xFF, 0xFF, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldx_absolute_reads_from_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xAE, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().x(), 0x8000);
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldx_direct_indexed_y_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA0, 0x33, 0x01,
                0xB6, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().x(), 0x8000);
        assert_eq!(cpu.registers().y(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldx_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA0, 0x00, 0x03, 0xBE, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().x(), 0x8000);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldy_absolute_reads_from_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xAC, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().y(), 0x8000);
        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldy_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA2, 0x33, 0x01,
                0xB4, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().y(), 0x8000);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ldy_absolute_indexed_x_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA2, 0x00, 0x03, 0xBC, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().y(), 0x8000);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn adc_absolute_and_absolute_long_read_memory_operands() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xED]);
        system.load(0x7EFFFF, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x12, 0x11, 0xE2, 0x20, 0x38, 0x6D, 0x33, 0x00, 0xC2,
                0x20, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9, 0x12, 0x11, 0xE2,
                0x20, 0x38, 0x6F, 0xFF, 0xFF, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_indexed_x_uses_direct_page_wrap_and_index() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0xCB, 0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x34, 0x12, 0xA2, 0x33, 0x01,
                0x38, 0x75, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn adc_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xCB, 0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA0, 0x00, 0x03, 0x38, 0x79, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn adc_absolute_indexed_x_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xCB, 0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x34, 0x12, 0xA2, 0x00, 0x03, 0x38, 0x7D, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn adc_absolute_long_indexed_x_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xCB, 0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x34, 0x12, 0xA2, 0x00, 0x03, 0x38, 0x7F, 0xFF, 0xFF,
                0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn and_direct_indexed_indirect_uses_direct_page_pointer_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0xFF, 0xFE, 0xA2, 0x91, 0xFF, 0x21, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0xEE5C);
        assert_eq!(cpu.registers().x(), 0xFF91);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn and_direct_indexed_x_uses_direct_page_wrap_and_index() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0xFF, 0xFE, 0xA2, 0x33, 0x01,
                0x35, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0xEE5C);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn and_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFE, 0xA0, 0x00, 0x03, 0x39, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0xEE5C);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn and_absolute_long_indexed_x_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFE, 0xA2, 0x00, 0x03, 0x3F, 0xFF, 0xFF, 0x7E,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0xEE5C);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_immediate_16bit_updates_negative() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFE, 0x49, 0x8C, 0x6F, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x9173);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_immediate_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xF0, 0x12, 0xE2, 0x20, 0x49, 0xAA, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x125A);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_direct_indexed_indirect_uses_direct_page_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0xFF, 0xFE, 0xA2, 0x91, 0xFF, 0x41, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x11A3);
        assert_eq!(cpu.registers().x(), 0xFF91);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_stack_relative_indirect_indexed_y_uses_stack_pointer_and_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x0001FF, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48,
                0xAB, 0xC2, 0x20, 0xA9, 0xFF, 0xFE, 0xA0, 0x00, 0x11, 0x53, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x11A3);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_direct_indirect_long_indexed_y_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE, 0x7E]);
        system.load(0x7F0FDC, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0xFF, 0xFE, 0xA0, 0x00, 0x11,
                0x57, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x11A3);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFE, 0xA0, 0x00, 0x03, 0x59, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x11A3);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn eor_absolute_long_indexed_x_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFE, 0xA2, 0x00, 0x03, 0x5F, 0xFF, 0xFF, 0x7E,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x11A3);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_immediate_16bit_updates_negative() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x00, 0x7F, 0x09, 0x5C, 0x80, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0xFF5C);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_immediate_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x50, 0x12, 0xE2, 0x20, 0x09, 0x0A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x125A);
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_direct_indexed_indirect_uses_direct_page_and_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x00FFA0, &[0x12, 0x12]);
        system.load(0x7F1212, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0xFF, 0xFE, 0xA2, 0x91, 0xFF, 0x01, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().x(), 0xFF91);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_direct_indexed_x_uses_direct_page_wrap_and_index() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x03, 0x10, 0xA2, 0x33, 0x01,
                0x15, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0xFF5F);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_stack_relative_indirect_indexed_y_uses_stack_pointer_and_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x0001FF, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48,
                0xAB, 0xC2, 0x20, 0xA9, 0x00, 0x7F, 0xA0, 0x00, 0x11, 0x13, 0x10, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0xFF5C);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_direct_indirect_long_indexed_y_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE, 0x7E]);
        system.load(0x7F0FDC, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x03, 0x10, 0xA0, 0x00, 0x11,
                0x17, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0xFF5F);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_absolute_indexed_y_carries_into_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0x00, 0x7F, 0xA0, 0x00, 0x03, 0x19, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0xFF5C);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ora_absolute_long_indexed_x_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x5C, 0xEF]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0xA2, 0x00, 0x03, 0xA9, 0x03, 0x10, 0x1F,
                0xFF, 0xFF, 0x7E, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0xFF5F);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn adc_direct_indirect_long_reads_full_24bit_pointer() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x34, 0x12, 0x7F]);
        system.load(0x7F1234, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x12, 0x11, 0xE2, 0x20, 0x38,
                0x67, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_indirect_reads_via_data_bank_pointer() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x34, 0x12]);
        system.load(0x7F1234, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7F, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0x12, 0x11, 0xE2, 0x20, 0x38, 0x72, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().db(), 0x7F);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_indirect_indexed_y_applies_y_offset_in_data_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x20, 0xA9,
                0xFF, 0xFF, 0x5B, 0xA9, 0x12, 0x11, 0xA0, 0x00, 0x11, 0xE2, 0x20, 0x38, 0x71, 0x34,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_stack_relative_indirect_indexed_y_uses_stack_pointer_and_offset() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x0001FF, &[0xDC, 0xFE]);
        system.load(0x7F0FDC, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48,
                0xAB, 0xC2, 0x20, 0xA9, 0x12, 0x11, 0xA0, 0x00, 0x11, 0xE2, 0x20, 0x38, 0x73, 0x10,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().s(), 0x01EF);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn adc_direct_indirect_long_indexed_y_carries_into_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xDC, 0xFE, 0x7E]);
        system.load(0x7F0FDC, &[0xED]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x12, 0x11, 0xA0, 0x00, 0x11,
                0xE2, 0x20, 0x38, 0x77, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(cpu.registers().a(), 0x1100);
        assert_eq!(cpu.registers().y(), 0x1100);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn wdm_immediate_is_a_two_byte_nop() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x31, 0xA9, 0x34, 0x12, 0xA2, 0x56, 0x34, 0xA0, 0x78, 0x56, 0x42,
                0xAB, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x1234);
        assert_eq!(cpu.registers().x(), 0x3456);
        assert_eq!(cpu.registers().y(), 0x5678);
        assert_eq!(cpu.registers().status().bits(), 0x04);
    }

    #[test]
    fn pea_pushes_immediate_word_onto_stack() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x18, 0xFB, 0xC2, 0x31, 0xF4, 0xCD, 0xAB, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().s(), 0x01FD);
        assert_eq!(system.memory.get(&0x0001FE), Some(&0xCD));
        assert_eq!(system.memory.get(&0x0001FF), Some(&0xAB));
    }

    #[test]
    fn pei_pushes_direct_indirect_word_from_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x65, 0x87]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x31, 0xA9, 0xFF, 0xFF, 0x5B, 0xD4, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert_eq!(cpu.registers().s(), 0x01FD);
        assert_eq!(system.memory.get(&0x0001FE), Some(&0x65));
        assert_eq!(system.memory.get(&0x0001FF), Some(&0x87));
    }

    #[test]
    fn per_pushes_signed_target_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x18, 0xFB, 0xC2, 0x31, 0x62, 0xFD, 0xFF, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 48);

        assert_eq!(cpu.registers().s(), 0x01FD);
        assert_eq!(system.memory.get(&0x0001FE), Some(&0x04));
        assert_eq!(system.memory.get(&0x0001FF), Some(&0x80));
    }

    #[test]
    fn phy_and_ply_round_trip_16bit_y() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x31, 0xA0, 0xDC, 0xFE, 0x5A, 0xA0, 0x00, 0x00, 0x7A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().y(), 0xFEDC);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn phk_pushes_program_bank_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x008000, &[0x18, 0xFB, 0xC2, 0x31, 0x5C, 0x00, 0x80, 0x7E]);
        system.load(0x7E8000, &[0xE2, 0x20, 0x4B, 0x68, 0xDB]);

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().pb(), 0x7E);
        assert_eq!(cpu.registers().a(), 0x007E);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn pld_pulls_direct_register_and_updates_flags() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x31, 0xF4, 0x53, 0x97, 0x2B, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().d(), 0x9753);
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn plp_restores_status_and_truncates_index_registers() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x31, 0xA2, 0x56, 0x34, 0xA0, 0x78, 0x56, 0xA9, 0x10, 0x00, 0xE2,
                0x20, 0x48, 0xC2, 0x20, 0x28, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().status().bits(), 0x10);
        assert_eq!(cpu.registers().x(), 0x0056);
        assert_eq!(cpu.registers().y(), 0x0078);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn php_and_phx_round_trip_status_and_index_values() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA2, 0x34, 0x12, 0xE2, 0x20, 0x08, 0x68, 0x85, 0x10, 0xC2,
                0x20, 0xDA, 0xFA, 0x86, 0x12, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x0010), Some(&0x25));
        assert_eq!(system.memory.get(&0x0012), Some(&0x34));
        assert_eq!(system.memory.get(&0x0013), Some(&0x12));
        assert_eq!(cpu.registers().x(), 0x1234);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn save_results_style_stack_and_transfer_ops_capture_state() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7E0000, &[0x9A, 0xBC]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x55, 0x44, 0xA2, 0x34, 0x12, 0xA0, 0x78, 0x56, 0x0B,
                0x48, 0xA9, 0x00, 0x00, 0x5B, 0x68, 0x85, 0x12, 0x86, 0x14, 0x84, 0x16, 0xFA, 0x86,
                0x1C, 0x3B, 0x1A, 0x1A, 0x1A, 0x85, 0x1A, 0xE2, 0x20, 0x8B, 0x68, 0x85, 0x1E, 0xA9,
                0x7E, 0x48, 0xAB, 0xC2, 0x20, 0xAF, 0x00, 0x00, 0x7E, 0xA6, 0x1C, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 160);

        assert_eq!(system.memory.get(&0x0012), Some(&0x55));
        assert_eq!(system.memory.get(&0x0013), Some(&0x44));
        assert_eq!(system.memory.get(&0x0014), Some(&0x34));
        assert_eq!(system.memory.get(&0x0015), Some(&0x12));
        assert_eq!(system.memory.get(&0x0016), Some(&0x78));
        assert_eq!(system.memory.get(&0x0017), Some(&0x56));
        assert_eq!(system.memory.get(&0x001A), Some(&0x02));
        assert_eq!(system.memory.get(&0x001B), Some(&0x02));
        assert_eq!(system.memory.get(&0x001C), Some(&0x00));
        assert_eq!(system.memory.get(&0x001D), Some(&0x00));
        assert_eq!(system.memory.get(&0x001E), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0xBC9A);
        assert_eq!(cpu.registers().d(), 0x0000);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0000);
        assert_eq!(cpu.registers().s(), 0x01FF);
    }

    #[test]
    fn asl_accumulator_16bit_sets_carry_and_zero() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x00, 0x80, 0x0A, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x0000);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn asl_accumulator_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x34, 0x12, 0xE2, 0x20, 0xA9, 0x81, 0x0A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x1202);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn asl_direct_16bit_wraps_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFF, 0x5B, 0x06, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(system.memory.get(&0x000033), Some(&0x00));
        assert_eq!(system.memory.get(&0x000034), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn asl_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0x16, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x00));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn asl_absolute_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x00]);
        system.load(0x7F0000, &[0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0x0E,
                0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7EFFFF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x00));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn rol_accumulator_16bit_rotates_carry_in_and_out() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x00, 0x80, 0x38, 0x2A, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x0001);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn rol_accumulator_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x34, 0x12, 0xE2, 0x20, 0xA9, 0x80, 0x18, 0x2A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x1200);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn rol_direct_16bit_wraps_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x11, 0x41]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFF, 0x5B, 0x38, 0x26, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(system.memory.get(&0x000033), Some(&0x23));
        assert_eq!(system.memory.get(&0x000034), Some(&0x82));
        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn rol_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x00, 0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0x38, 0x36, 0x02,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x01));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn rol_absolute_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x11]);
        system.load(0x7F0000, &[0x41]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0x38,
                0x2E, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7EFFFF), Some(&0x23));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x82));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn rol_absolute_indexed_x_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x00]);
        system.load(0x7F0300, &[0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0xA2,
                0x00, 0x03, 0x18, 0x3E, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(system.memory.get(&0x7F02FF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0x00));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ror_accumulator_16bit_rotates_carry_in_and_out() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x01, 0x00, 0x38, 0x6A, 0xDB],
        );

        run_until_stopped(&mut cpu, &mut system, 64);

        assert_eq!(cpu.registers().a(), 0x8000);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn ror_accumulator_8bit_preserves_high_byte() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x34, 0x12, 0xE2, 0x20, 0xA9, 0x01, 0x18, 0x6A, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x1200);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn ror_direct_16bit_wraps_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x22, 0x42]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFF, 0x5B, 0x38, 0x66, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(system.memory.get(&0x000033), Some(&0x11));
        assert_eq!(system.memory.get(&0x000034), Some(&0xA1));
        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn ror_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x01, 0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0x18, 0x76, 0x02,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x00));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn ror_absolute_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x22]);
        system.load(0x7F0000, &[0x42]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0x38,
                0x6E, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7EFFFF), Some(&0x11));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0xA1));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(!cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn ror_absolute_indexed_x_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x01]);
        system.load(0x7F0300, &[0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0xA2,
                0x00, 0x03, 0x18, 0x7E, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(system.memory.get(&0x7F02FF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0x00));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn lsr_direct_16bit_wraps_direct_page() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x01, 0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFF, 0x5B, 0x46, 0x34, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(system.memory.get(&0x000033), Some(&0x00));
        assert_eq!(system.memory.get(&0x000034), Some(&0x00));
        assert_eq!(cpu.registers().a(), 0xFFFF);
        assert_eq!(cpu.registers().d(), 0xFFFF);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn lsr_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0x03, 0x00]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0x56, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(system.memory.get(&0x000134), Some(&0x01));
        assert_eq!(system.memory.get(&0x000135), Some(&0x00));
        assert_eq!(cpu.registers().x(), 0x0133);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn lsr_absolute_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7EFFFF, &[0x01]);
        system.load(0x7F0000, &[0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0x4E,
                0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7EFFFF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0000), Some(&0x40));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn lsr_absolute_indexed_x_carries_into_next_bank() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0x01]);
        system.load(0x7F0300, &[0x80]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0xA2,
                0x00, 0x03, 0x5E, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 128);

        assert_eq!(system.memory.get(&0x7F02FF), Some(&0x00));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0x40));
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
    }

    #[test]
    fn bit_direct_16bit_updates_flags_without_writing_memory() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0x34, 0x52]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0xFF, 0xFF, 0x5B, 0xA9, 0x77, 0x93, 0x38, 0x24, 0x34,
                0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 96);

        assert_eq!(cpu.registers().a(), 0x9377);
        assert_eq!(system.memory.get(&0x000033), Some(&0x34));
        assert_eq!(system.memory.get(&0x000034), Some(&0x52));
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::OVERFLOW));
        assert!(!cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn bit_direct_8bit_uses_low_byte_for_n_and_v() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000033, &[0xC0]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x20, 0xA9, 0x40, 0x12, 0xE2, 0x20, 0x38, 0x24, 0x33, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 80);

        assert_eq!(cpu.registers().a(), 0x1240);
        assert_eq!(system.memory.get(&0x000033), Some(&0xC0));
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(cpu.registers().status().contains(CpuStatus::OVERFLOW));
        assert!(!cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn bit_direct_indexed_x_uses_wrapped_direct_page_address() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x000134, &[0xAA, 0xAA]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0xFF, 0xFF, 0x5B, 0xA2, 0x33, 0x01, 0xA9, 0x55, 0x55,
                0x38, 0x34, 0x02, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 112);

        assert_eq!(cpu.registers().a(), 0x5555);
        assert_eq!(cpu.registers().x(), 0x0133);
        assert_eq!(system.memory.get(&0x000134), Some(&0xAA));
        assert_eq!(system.memory.get(&0x000135), Some(&0xAA));
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::OVERFLOW));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }

    #[test]
    fn bit_absolute_indexed_x_reads_across_bank_boundary() {
        let mut cpu = Cpu::new();
        let mut system = TestBus::with_reset_vector(0x8000);
        system.load(0x7F02FF, &[0xAA]);
        system.load(0x7F0300, &[0xAA]);
        system.load(
            0x008000,
            &[
                0x18, 0xFB, 0xC2, 0x30, 0xA9, 0x7E, 0x00, 0xE2, 0x20, 0x48, 0xAB, 0xC2, 0x30, 0xA2,
                0x00, 0x03, 0xA9, 0x55, 0x55, 0x38, 0x3C, 0xFF, 0xFF, 0xDB,
            ],
        );

        run_until_stopped(&mut cpu, &mut system, 144);

        assert_eq!(cpu.registers().a(), 0x5555);
        assert_eq!(cpu.registers().db(), 0x7E);
        assert_eq!(cpu.registers().x(), 0x0300);
        assert_eq!(system.memory.get(&0x7F02FF), Some(&0xAA));
        assert_eq!(system.memory.get(&0x7F0300), Some(&0xAA));
        assert!(cpu.registers().status().contains(CpuStatus::CARRY));
        assert!(cpu.registers().status().contains(CpuStatus::NEGATIVE));
        assert!(!cpu.registers().status().contains(CpuStatus::OVERFLOW));
        assert!(cpu.registers().status().contains(CpuStatus::ZERO));
    }
}
