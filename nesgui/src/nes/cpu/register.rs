// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Register {
    pc: u16,
    sp: u8,
    a: u8,
    x: u8,
    y: u8,

    c: bool, // 0x01
    z: bool, // 0x02
    i: bool, // 0x04
    d: bool, // 0x08
    b: bool, // 0x10
    r: bool, // 0x20
    v: bool, // 0x40
    n: bool, // 0x80
}

impl Register {
    pub fn new() -> Self {
        Self {
            pc: 0,
            sp: 0,
            a: 0,
            x: 0,
            y: 0,

            c: false, // 0x01
            z: false, // 0x02
            i: true,  // 0x04
            d: false, // 0x08
            b: true, // 0x10
            r: true,  // 0x20
            v: false, // 0x40
            n: false, // 0x80
        }
    }

    pub fn get_pc(&self) -> u16 {
        self.pc
    }
    pub fn get_sp(&self) -> u8 {
        self.sp
    }
    pub fn get_a(&self) -> u8 {
        self.a
    }
    pub fn get_x(&self) -> u8 {
        self.x
    }
    pub fn get_y(&self) -> u8 {
        self.y
    }
    pub fn get_c(&self) -> bool {
        self.c
    }
    pub fn get_z(&self) -> bool {
        self.z
    }
    pub fn get_i(&self) -> bool {
        self.i
    }
    pub fn get_v(&self) -> bool {
        self.v
    }
    // pub fn get_b(&self) -> bool {
    //     self.b
    // }
    pub fn get_n(&self) -> bool {
        self.n
    }
    pub fn get_p(&self) -> u8 {
        (if self.c { 0x01 } else { 0 })
            | (if self.z { 0x02 } else { 0 })
            | (if self.i { 0x04 } else { 0 })
            | (if self.d { 0x08 } else { 0 })
            | (if self.b { 0x10 } else { 0 })
            | (if self.r { 0x20 } else { 0 })
            | (if self.v { 0x40 } else { 0 })
            | (if self.n { 0x80 } else { 0 })
    }

    pub fn set_pc(&mut self, value: u16) {
        self.pc = value;
    }
    pub fn set_sp(&mut self, value: u8) {
        self.sp = value;
    }
    pub fn set_a(&mut self, value: u8) {
        self.a = value;
    }
    pub fn set_x(&mut self, value: u8) {
        self.x = value;
    }
    pub fn set_y(&mut self, value: u8) {
        self.y = value;
    }
    pub fn set_c(&mut self, value: bool) {
        self.c = value;
    }
    pub fn set_i(&mut self, value: bool) {
        self.i = value;
    }
    pub fn set_d(&mut self, value: bool) {
        self.d = value;
    }
    pub fn set_b(&mut self, value: bool) {
        self.b = value;
    }
    pub fn set_v(&mut self, value: bool) {
        self.v = value;
    }
    pub fn set_p(&mut self, value: u8) {
        self.c = value & 0x01 == 0x01;
        self.z = value & 0x02 == 0x02;
        self.i = value & 0x04 == 0x04;
        self.d = value & 0x08 == 0x08;
        self.b = value & 0x10 == 0x10;
        self.r = value & 0x20 == 0x20;
        self.v = value & 0x40 == 0x40;
        self.n = value & 0x80 == 0x80;
    }

    pub fn set_z_from_value(&mut self, value: u8) {
        self.z = value == 0;
    }

    pub fn set_n_from_value(&mut self, value: u8) {
        self.n = value & 0x80 != 0;
    }

    pub fn set_nz_from_value(&mut self, value: u8) {
        self.set_z_from_value(value);
        self.set_n_from_value(value);
    }
}
