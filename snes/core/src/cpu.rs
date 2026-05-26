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
    Sec,
    Sei,
    Xce,
    Txs,
    Stp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Immediate8Op {
    Rep,
    Sep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmediateLoadTarget {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbsoluteOp {
    Sta { wide: bool },
    Stz { wide: bool },
    Jmp,
    Jsr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicroState {
    Reset { remaining: u8, low: u8 },
    Fetch,
    Implied(ImpliedOp),
    Immediate8(Immediate8Op),
    ImmediateLoadLow(ImmediateLoadTarget),
    ImmediateLoadHigh(ImmediateLoadTarget, u8),
    AbsoluteLow(AbsoluteOp),
    AbsoluteHigh(AbsoluteOp, u8),
    AbsoluteWriteHigh { address: u16, value: u8 },
    JsrPushHigh { target: u16, return_addr: u16 },
    JsrPushLow { target: u16, return_addr: u16 },
    RtsPullLow,
    RtsPullHigh(u8),
    RtsFinalize,
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
            MicroState::AbsoluteLow(op) => self.execute_absolute_low(bus, op),
            MicroState::AbsoluteHigh(op, low) => self.execute_absolute_high(bus, op, low),
            MicroState::AbsoluteWriteHigh { address, value } => {
                self.write_bus(bus, address, value);
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
            ImpliedOp::Sec => self.registers.p.insert(CpuStatus::CARRY),
            ImpliedOp::Sei => self.registers.p.insert(CpuStatus::IRQ_DISABLE),
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

    fn execute_immediate8(&mut self, bus: &mut dyn CpuBus, op: Immediate8Op) {
        let value = bus.read(self.full_pc());
        self.registers.pc = self.registers.pc.wrapping_add(1);
        match op {
            Immediate8Op::Rep => self.apply_status_mask(value, false),
            Immediate8Op::Sep => self.apply_status_mask(value, true),
        }
        self.micro_state = MicroState::Fetch;
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
            AbsoluteOp::Sta { wide } => {
                self.write_bus(bus, address, self.registers.a as u8);
                if wide {
                    self.micro_state = MicroState::AbsoluteWriteHigh {
                        address: address.wrapping_add(1),
                        value: (self.registers.a >> 8) as u8,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Stz { wide } => {
                self.write_bus(bus, address, 0);
                if wide {
                    self.micro_state = MicroState::AbsoluteWriteHigh {
                        address: address.wrapping_add(1),
                        value: 0,
                    };
                } else {
                    self.micro_state = MicroState::Fetch;
                }
            }
            AbsoluteOp::Jmp => {
                self.registers.pc = address;
                self.micro_state = MicroState::Fetch;
            }
            AbsoluteOp::Jsr => {
                let return_addr = self.registers.pc.wrapping_sub(1);
                self.micro_state = MicroState::JsrPushHigh {
                    target: address,
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
                    self.registers.a = (self.registers.a & 0xFF00) | (value & 0x00FF);
                } else {
                    self.registers.a = value;
                }
            }
            ImmediateLoadTarget::X => {
                if self.index_is_8bit() {
                    self.registers.x = value & 0x00FF;
                } else {
                    self.registers.x = value;
                }
            }
            ImmediateLoadTarget::Y => {
                if self.index_is_8bit() {
                    self.registers.y = value & 0x00FF;
                } else {
                    self.registers.y = value;
                }
            }
        }
    }

    fn decode_opcode(&mut self, opcode: u8) -> MicroState {
        match opcode {
            0xEA => MicroState::Implied(ImpliedOp::Nop),
            0x18 => MicroState::Implied(ImpliedOp::Clc),
            0x38 => MicroState::Implied(ImpliedOp::Sec),
            0x78 => MicroState::Implied(ImpliedOp::Sei),
            0xFB => MicroState::Implied(ImpliedOp::Xce),
            0x9A => MicroState::Implied(ImpliedOp::Txs),
            0xDB => MicroState::Implied(ImpliedOp::Stp),
            0xC2 => MicroState::Immediate8(Immediate8Op::Rep),
            0xE2 => MicroState::Immediate8(Immediate8Op::Sep),
            0xA9 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::A),
            0xA2 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::X),
            0xA0 => MicroState::ImmediateLoadLow(ImmediateLoadTarget::Y),
            0x8D => MicroState::AbsoluteLow(AbsoluteOp::Sta {
                wide: !self.accumulator_is_8bit(),
            }),
            0x9C => MicroState::AbsoluteLow(AbsoluteOp::Stz {
                wide: !self.accumulator_is_8bit(),
            }),
            0x4C => MicroState::AbsoluteLow(AbsoluteOp::Jmp),
            0x20 => MicroState::AbsoluteLow(AbsoluteOp::Jsr),
            0x60 => MicroState::RtsPullLow,
            _ => MicroState::Stopped,
        }
    }

    fn full_pc(&self) -> u32 {
        ((self.registers.pb as u32) << 16) | (self.registers.pc as u32)
    }

    fn write_bus(&mut self, bus: &mut dyn CpuBus, address: u16, value: u8) {
        let full = ((self.registers.db as u32) << 16) | u32::from(address);
        bus.write(full, value);
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
}
