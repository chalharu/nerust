// Copyright (c) 2019 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Hash, Debug, EnumIter)]
pub(crate) enum CpuStatesEnum {
    FetchOpCode,
    Reset,
    Irq,
    AbsoluteIndirect,
    AbsoluteXRMW,
    AbsoluteX,
    AbsoluteYRMW,
    AbsoluteY,
    Absolute,
    Accumulator,
    Immediate,
    Implied,
    IndexedIndirect,
    IndirectIndexedRMW,
    IndirectIndexed,
    Relative,
    ZeroPageX,
    ZeroPageY,
    ZeroPage,
    And,
    Eor,
    Ora,
    Adc,
    Sbc,
    Bit,
    Lax,
    Anc,
    Alr,
    Arr,
    Xaa,
    Las,
    Axs,
    Sax,
    Tas,
    Ahx,
    Shx,
    Shy,
    Cmp,
    Cpx,
    Cpy,
    Bcc,
    Bcs,
    Beq,
    Bmi,
    Bne,
    Bpl,
    Bvc,
    Bvs,
    Dex,
    Dey,
    Dec,
    Clc,
    Cld,
    Cli,
    Clv,
    Sec,
    Sed,
    Sei,
    Inx,
    Iny,
    Inc,
    Brk,
    Rti,
    Rts,
    Jmp,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Nop,
    Kil,
    Isc,
    Dcp,
    Slo,
    Rla,
    Sre,
    Rra,
    AslAcc,
    AslMem,
    LsrAcc,
    LsrMem,
    RolAcc,
    RolMem,
    RorAcc,
    RorMem,
    Pla,
    Plp,
    Pha,
    Php,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Tsx,
    Txa,
    Tya,
    Txs,
}

impl Default for CpuStatesEnum {
    fn default() -> Self {
        CpuStatesEnum::Reset
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct InternalStat {
    opcode: usize,
    address: usize,
    step: usize,
    tempaddr: usize,
    data: u8,
    crossed: bool,
    interrupt: bool,
    state: CpuStatesEnum,
}

impl InternalStat {
    pub fn new() -> Self {
        Self {
            opcode: 0,
            address: 0,
            step: 0,
            tempaddr: 0,
            data: 0,
            crossed: false,
            interrupt: false,
            state: CpuStatesEnum::Reset,
        }
    }

    pub fn reset(&mut self) {
        self.step = 0;
        self.state = CpuStatesEnum::Reset;
    }

    pub fn set_opcode(&mut self, value: usize) {
        self.opcode = value;
    }

    pub fn get_opcode(&self) -> usize {
        self.opcode
    }

    pub fn set_address(&mut self, value: usize) {
        self.address = value;
    }

    pub fn get_address(&self) -> usize {
        self.address
    }

    pub fn set_step(&mut self, value: usize) {
        self.step = value;
    }

    pub fn get_step(&self) -> usize {
        self.step
    }

    pub fn set_tempaddr(&mut self, value: usize) {
        self.tempaddr = value;
    }

    pub fn get_tempaddr(&self) -> usize {
        self.tempaddr
    }

    pub fn set_data(&mut self, value: u8) {
        self.data = value;
    }

    pub fn get_data(&self) -> u8 {
        self.data
    }

    pub fn set_interrupt(&mut self, value: bool) {
        self.interrupt = value;
    }

    pub fn get_interrupt(&self) -> bool {
        self.interrupt
    }

    pub fn set_crossed(&mut self, value: bool) {
        self.crossed = value;
    }

    pub fn get_crossed(&self) -> bool {
        self.crossed
    }

    pub fn set_state(&mut self, value: CpuStatesEnum) {
        self.state = value;
    }

    pub fn get_state(&self) -> CpuStatesEnum {
        self.state
    }
}
