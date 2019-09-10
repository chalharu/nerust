// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) trait Read {
    fn reader(register: &mut Register, value: u8);

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let data = core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                Self::reader(&mut core.register, data);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! read {
    ($name:ident, $reader:expr) => {
        pub(crate) struct $name;

        impl Read for $name {
            fn reader(register: &mut Register, value: u8) {
                $reader(register, value);
            }
        }

        cpu_step_state_impl!($name);
    };
}

pub(crate) trait Write {
    fn writer(register: &mut Register, internal_stat: &InternalStat) -> (u8, usize);

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let (data, address) = Self::writer(&mut core.register, &core.internal_stat);
                core.memory.write(
                    address,
                    data,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! write {
    ($name:ident, $writer:expr) => {
        pub(crate) struct $name;

        impl Write for $name {
            fn writer(register: &mut Register, internal_stat: &InternalStat) -> (u8, usize) {
                $writer(register, internal_stat)
            }
        }

        cpu_step_state_impl!($name);
    };
}

read!(Lax, |r: &mut Register, v: u8| {
    r.set_a(v);
    r.set_x(v);
    r.set_nz_from_value(v);
});

read!(Anc, |r: &mut Register, v: u8| {
    let result = r.get_a() & v;
    r.set_nz_from_value(result);
    r.set_c(result & 0x80 != 0);
    r.set_a(result);
});

read!(Alr, |r: &mut Register, v: u8| {
    let result = r.get_a() & v;
    r.set_c(result & 0x01 != 0);
    let value = result >> 1;
    r.set_nz_from_value(value);
    r.set_a(value);
});

read!(Arr, |r: &mut Register, v: u8| {
    let result = r.get_a() & v;
    let value = result >> 1 | if r.get_c() { 0x80 } else { 0 };
    r.set_c(result & 0x80 != 0);
    r.set_v((((value >> 6) & 1) != 0) ^ (((value >> 5) & 1) != 0));
    r.set_nz_from_value(value);
    r.set_a(value);
});

read!(Xaa, |r: &mut Register, v: u8| {
    let result = r.get_x() & v;
    r.set_nz_from_value(result);
    r.set_a(result);
});

read!(Las, |r: &mut Register, v: u8| {
    let result = r.get_sp() & v;
    r.set_nz_from_value(result);
    r.set_a(result);
    r.set_x(result);
    r.set_sp(result);
});

read!(Axs, |r: &mut Register, v: u8| {
    let a = u16::from(r.get_a() & r.get_x());
    let b = u16::from(v);
    let d = a.wrapping_sub(b);

    let result = (d & 0xFF) as u8;
    r.set_nz_from_value(result);
    r.set_x(result);
    r.set_c(d <= 0xFF);
});

write!(Sax, |r: &mut Register, i: &InternalStat| (
    r.get_a() & r.get_x(),
    i.get_address()
));

write!(Tas, |r: &mut Register, i: &InternalStat| {
    let sp = r.get_a() & r.get_x();
    r.set_sp(sp);
    (
        sp & ((r.get_pc() >> 8) as u8).wrapping_add(1),
        i.get_address(),
    )
});

write!(Ahx, |r: &mut Register, i: &InternalStat| {
    let address = i.get_address();
    let high = ((address >> 8) as u8).wrapping_add(1);
    (r.get_a() & r.get_x() & high, address)
});

write!(Shx, |r: &mut Register, i: &InternalStat| {
    let address = i.get_address();
    let high = ((address >> 8) as u8).wrapping_add(1);
    let low = address & 0xFF;
    let value = r.get_x() & high;
    let new_addr = (usize::from(value) << 8) | low;
    (value, new_addr)
});

write!(Shy, |r: &mut Register, i: &InternalStat| {
    let address = i.get_address();
    let high = ((address >> 8) as u8).wrapping_add(1);
    let low = address & 0xFF;
    let value = r.get_y() & high;
    let new_addr = (usize::from(value) << 8) | low;
    (value, new_addr)
});
