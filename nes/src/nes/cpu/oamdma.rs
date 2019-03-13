// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum OamDmaStateEnumValue {
    Step0,
    Step1,
    Step2,
    None,
}

pub(crate) struct OamDmaStateValue {
    offset: u8,
    count: u8,
    value: u8,
}

impl OamDmaStateValue {
    pub fn new() -> Self {
        Self {
            offset: 0,
            count: 0,
            value: 0,
        }
    }
}

pub(crate) struct OamDmaState {
    state_pool: [Box<OamDmaStepState>; OamDmaStateEnumValue::None as usize],
    state: OamDmaStateEnumValue,
    value: OamDmaStateValue,
}

impl OamDmaState {
    pub fn new() -> OamDmaState {
        Self {
            state_pool: [
                Box::new(OamDma),
                Box::new(OamDmaStep1),
                Box::new(OamDmaStep2),
            ],
            state: OamDmaStateEnumValue::None,
            value: OamDmaStateValue::new(),
        }
    }

    pub fn has_transaction(&self) -> bool {
        self.state != OamDmaStateEnumValue::None
    }

    pub fn start_transaction(&mut self, offset: u8) {
        self.state = OamDmaStateEnumValue::Step0;
        self.value.offset = offset;
        self.value.count = 255;
    }

    pub fn reset(&mut self) {
        self.state = OamDmaStateEnumValue::None;
    }

    pub fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) {
        self.state = self.state_pool[self.state as usize].next(
            core,
            ppu,
            cartridge,
            controller,
            apu,
            &mut self.value,
        );
    }

    pub fn count(&self) -> Option<u8> {
        if self.state == OamDmaStateEnumValue::None {
            None
        } else {
            Some(self.value.count)
        }
    }
}

pub(crate) trait OamDmaStepState {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue;
}

pub(crate) struct OamDma;

impl OamDmaStepState for OamDma {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
        _value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        // dummy read
        read_dummy_current(core, ppu, cartridge, controller, apu);
        if core.cycles & 1 != 0 {
            OamDmaStateEnumValue::Step0
        } else {
            OamDmaStateEnumValue::Step1
        }
    }
}

struct OamDmaStep1;

impl OamDmaStepState for OamDmaStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        value.value = core.memory.read(
            usize::from(value.offset) * 0x100 + usize::from(255 - value.count),
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        OamDmaStateEnumValue::Step2
    }
}

struct OamDmaStep2;

impl OamDmaStepState for OamDmaStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        core.memory.write(
            0x2004,
            value.value,
            ppu,
            cartridge,
            controller,
            apu,
            &mut core.interrupt,
        );
        if value.count == 0 {
            OamDmaStateEnumValue::None
        } else {
            value.count -= 1;
            OamDmaStateEnumValue::Step1
        }
    }
}
