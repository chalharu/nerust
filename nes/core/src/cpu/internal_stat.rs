// Copyright (c) 2019 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Eq,
    Clone,
    Copy,
    Hash,
    Debug,
    Default,
    strum_macros::EnumIter,
)]
pub(crate) enum CpuStatesEnum {
    FetchOpCode,
    #[default]
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

impl CpuStatesEnum {
    pub(crate) const COUNT: usize = CpuStatesEnum::Txs as usize + 1;
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub(crate) struct InternalStat {
    pub(super) opcode: usize,
    pub(super) address: usize,
    pub(super) step: usize,
    pub(super) tempaddr: usize,
    pub(super) data: u8,
    pub(super) crossed: bool,
    pub(super) interrupt: bool,
    pub(super) state: CpuStatesEnum,
}

impl InternalStat {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn reset(&mut self) {
        self.step = 0;
        self.state = CpuStatesEnum::Reset;
    }

    pub(crate) fn set_opcode(&mut self, value: usize) {
        self.opcode = value;
    }

    pub(crate) fn get_opcode(&self) -> usize {
        self.opcode
    }

    pub(crate) fn validate(&self) -> Result<(), PersistenceError> {
        if self.opcode >= 0x100 {
            return Err(PersistenceError::Validation("CPU opcode overflow".into()));
        }
        if self.address > u16::MAX as usize {
            return Err(PersistenceError::Validation("CPU address overflow".into()));
        }
        if self.tempaddr > u16::MAX as usize {
            return Err(PersistenceError::Validation(
                "CPU temporary address overflow".into(),
            ));
        }
        if self.step > 8 {
            return Err(PersistenceError::Validation("CPU step overflow".into()));
        }
        Ok(())
    }

    pub(crate) fn set_address(&mut self, value: usize) {
        self.address = value;
    }

    pub(crate) fn get_address(&self) -> usize {
        self.address
    }

    pub(crate) fn set_step(&mut self, value: usize) {
        self.step = value;
    }

    pub(crate) fn get_step(&self) -> usize {
        self.step
    }

    pub(crate) fn set_tempaddr(&mut self, value: usize) {
        self.tempaddr = value;
    }

    pub(crate) fn get_tempaddr(&self) -> usize {
        self.tempaddr
    }

    pub(crate) fn set_data(&mut self, value: u8) {
        self.data = value;
    }

    pub(crate) fn get_data(&self) -> u8 {
        self.data
    }

    pub(crate) fn set_interrupt(&mut self, value: bool) {
        self.interrupt = value;
    }

    pub(crate) fn get_interrupt(&self) -> bool {
        self.interrupt
    }

    pub(crate) fn set_crossed(&mut self, value: bool) {
        self.crossed = value;
    }

    pub(crate) fn get_crossed(&self) -> bool {
        self.crossed
    }

    pub(crate) fn get_state(&self) -> CpuStatesEnum {
        self.state
    }
}

impl TryFrom<usize> for CpuStatesEnum {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        use CpuStatesEnum::*;
        Ok(match value {
            0 => FetchOpCode,
            1 => Reset,
            2 => Irq,
            3 => AbsoluteIndirect,
            4 => AbsoluteXRMW,
            5 => AbsoluteX,
            6 => AbsoluteYRMW,
            7 => AbsoluteY,
            8 => Absolute,
            9 => Accumulator,
            10 => Immediate,
            11 => Implied,
            12 => IndexedIndirect,
            13 => IndirectIndexedRMW,
            14 => IndirectIndexed,
            15 => Relative,
            16 => ZeroPageX,
            17 => ZeroPageY,
            18 => ZeroPage,
            19 => And,
            20 => Eor,
            21 => Ora,
            22 => Adc,
            23 => Sbc,
            24 => Bit,
            25 => Lax,
            26 => Anc,
            27 => Alr,
            28 => Arr,
            29 => Xaa,
            30 => Las,
            31 => Axs,
            32 => Sax,
            33 => Tas,
            34 => Ahx,
            35 => Shx,
            36 => Shy,
            37 => Cmp,
            38 => Cpx,
            39 => Cpy,
            40 => Bcc,
            41 => Bcs,
            42 => Beq,
            43 => Bmi,
            44 => Bne,
            45 => Bpl,
            46 => Bvc,
            47 => Bvs,
            48 => Dex,
            49 => Dey,
            50 => Dec,
            51 => Clc,
            52 => Cld,
            53 => Cli,
            54 => Clv,
            55 => Sec,
            56 => Sed,
            57 => Sei,
            58 => Inx,
            59 => Iny,
            60 => Inc,
            61 => Brk,
            62 => Rti,
            63 => Rts,
            64 => Jmp,
            65 => Jsr,
            66 => Lda,
            67 => Ldx,
            68 => Ldy,
            69 => Nop,
            70 => Kil,
            71 => Isc,
            72 => Dcp,
            73 => Slo,
            74 => Rla,
            75 => Sre,
            76 => Rra,
            77 => AslAcc,
            78 => AslMem,
            79 => LsrAcc,
            80 => LsrMem,
            81 => RolAcc,
            82 => RolMem,
            83 => RorAcc,
            84 => RorMem,
            85 => Pla,
            86 => Plp,
            87 => Pha,
            88 => Php,
            89 => Sta,
            90 => Stx,
            91 => Sty,
            92 => Tax,
            93 => Tay,
            94 => Tsx,
            95 => Txa,
            96 => Tya,
            97 => Txs,
            _ => return Err(()),
        })
    }
}
use crate::persistence_error::PersistenceError;
