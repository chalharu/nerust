// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Address {
    pub address: usize,
    pub cycles: usize,
}

pub(crate) trait AddressingMode {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address;
    fn name(&self) -> &'static str;
    fn opcode_length(&self) -> u16;
}

pub(crate) struct Absolute;
impl AddressingMode for Absolute {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        Address {
            address: memory.read_u16(pc as usize) as usize,
            cycles: 3,
        }
    }
    fn name(&self) -> &'static str {
        "Absolute"
    }
    fn opcode_length(&self) -> u16 {
        3
    }
}

pub(crate) struct AbsoluteX;
impl AddressingMode for AbsoluteX {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory.read_u16(pc as usize);
        let new_address = address.wrapping_add(u16::from(state.register().get_x()));
        Address {
            address: new_address as usize,
            cycles: 3 + if page_crossed(address, new_address) {
                1
            } else {
                0
            },
        }
    }
    fn name(&self) -> &'static str {
        "AbsoluteX"
    }
    fn opcode_length(&self) -> u16 {
        3
    }
}

pub(crate) struct AbsoluteY;
impl AddressingMode for AbsoluteY {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory.read_u16(pc as usize);
        let new_address = address.wrapping_add(u16::from(state.register().get_y()));
        Address {
            address: new_address as usize,
            cycles: 3 + if page_crossed(address, new_address) {
                1
            } else {
                0
            },
        }
    }
    fn name(&self) -> &'static str {
        "AbsoluteY"
    }
    fn opcode_length(&self) -> u16 {
        3
    }
}

pub(crate) struct Accumulator;
impl AddressingMode for Accumulator {
    fn execute(&self, _state: &mut State, _memory: &mut Memory) -> Address {
        Address {
            address: 0,
            cycles: 1,
        }
    }
    fn name(&self) -> &'static str {
        "Accumulator"
    }
    fn opcode_length(&self) -> u16 {
        1
    }
}

pub(crate) struct Immediate;
impl AddressingMode for Immediate {
    fn execute(&self, state: &mut State, _memory: &mut Memory) -> Address {
        Address {
            address: state.register().get_pc().wrapping_add(1) as usize,
            cycles: 1,
        }
    }
    fn name(&self) -> &'static str {
        "Immediate"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct Implied;
impl AddressingMode for Implied {
    fn execute(&self, _state: &mut State, _memory: &mut Memory) -> Address {
        Address {
            address: 0,
            cycles: 1,
        }
    }
    fn name(&self) -> &'static str {
        "Implied"
    }
    fn opcode_length(&self) -> u16 {
        1
    }
}

pub(crate) struct IndexedIndirect;
impl AddressingMode for IndexedIndirect {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory.read(pc as usize);
        let new_address = address.wrapping_add(state.register().get_x());
        Address {
            address: memory.read_u16_bug(usize::from(new_address)) as usize,
            cycles: 5,
        }
    }
    fn name(&self) -> &'static str {
        "IndexedIndirect"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct AbsoluteIndirect;
impl AddressingMode for AbsoluteIndirect {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory.read_u16(pc as usize);
        Address {
            address: memory.read_u16_bug(address as usize) as usize,
            cycles: 5,
        }
    }
    fn name(&self) -> &'static str {
        "AbsoluteIndirect"
    }
    fn opcode_length(&self) -> u16 {
        3
    }
}

pub(crate) struct IndirectIndexed;
impl AddressingMode for IndirectIndexed {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = u16::from(memory.read(state.register().get_pc().wrapping_add(1) as usize));
        let address = memory.read_u16_bug(pc as usize);
        let new_address = address.wrapping_add(u16::from(state.register().get_y()));
        Address {
            address: new_address as usize,
            cycles: 4 + if page_crossed(address, new_address) {
                1
            } else {
                0
            },
        }
    }
    fn name(&self) -> &'static str {
        "IndirectIndexed"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct Relative;
impl AddressingMode for Relative {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let offset = u16::from(memory.read(pc as usize));
        let address = pc
            .wrapping_add(1)
            .wrapping_add(offset)
            .wrapping_sub(if offset < 0x80 { 0 } else { 0x100 });
        Address {
            address: address as usize,
            cycles: 1,
        }
    }
    fn name(&self) -> &'static str {
        "Relative"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct ZeroPage;
impl AddressingMode for ZeroPage {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory.read(pc as usize);
        Address {
            address: usize::from(address),
            cycles: 2,
        }
    }
    fn name(&self) -> &'static str {
        "ZeroPage"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct ZeroPageX;
impl AddressingMode for ZeroPageX {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory
            .read(pc as usize)
            .wrapping_add(state.register().get_x());
        Address {
            address: usize::from(address),
            cycles: 3,
        }
    }
    fn name(&self) -> &'static str {
        "ZeroPageX"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}

pub(crate) struct ZeroPageY;
impl AddressingMode for ZeroPageY {
    fn execute(&self, state: &mut State, memory: &mut Memory) -> Address {
        let pc = state.register().get_pc().wrapping_add(1);
        let address = memory
            .read(pc as usize)
            .wrapping_add(state.register().get_y());
        Address {
            address: usize::from(address),
            cycles: 3,
        }
    }
    fn name(&self) -> &'static str {
        "ZeroPageY"
    }
    fn opcode_length(&self) -> u16 {
        2
    }
}
