// Copyright (c) 2019 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct InternalStat {
    opcode: usize,
    address: usize,
    step: usize,
    tempaddr: usize,
    data: u8,
    crossed: bool,
    interrupt: bool,
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
        }
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
}
