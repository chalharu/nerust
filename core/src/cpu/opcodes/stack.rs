use super::super::CpuCartridgeBus;
use super::super::{
    Apu, Controller, Core, CpuStepStateEnum, Ppu, Register, RegisterP, pull, push,
    read_dummy_current,
};
use super::exit_opcode;

pub(crate) trait Pull {
    fn setter(register: &mut Register, value: u8);

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            2 => {
                // dummy read
                let sp = core.register.get_sp();
                let _ = core.memory.read(
                    0x100 | usize::from(sp),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                let value = pull(core, ppu, cartridge, controller, apu);
                Self::setter(&mut core.register, value);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! pull {
    ($name:ident, $func:expr) => {
        pub(crate) struct $name;

        impl Pull for $name {
            fn setter(register: &mut Register, value: u8) {
                $func(register, value);
            }
        }

        cpu_step_state_impl!($name);
    };
}

pull!(Pla, |r: &mut Register, v: u8| {
    r.set_a(v);
    r.set_nz_from_value(v);
});

pull!(Plp, |r: &mut Register, v: u8| r.set_p(
    (v & !(RegisterP::BREAK.bits())) | RegisterP::RESERVED.bits()
));

pub(crate) trait Push {
    fn getter(register: &Register) -> u8;

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
                core.internal_stat.set_data(Self::getter(&core.register));
            }
            2 => {
                push(
                    core,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    core.internal_stat.get_data(),
                );
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! push {
    ($name:ident, $func:expr) => {
        pub(crate) struct $name;

        impl Push for $name {
            fn getter(register: &Register) -> u8 {
                $func(register)
            }
        }

        cpu_step_state_impl!($name);
    };
}

push!(Pha, Register::get_a);
push!(Php, |r: &Register| r.get_p() | 0x10);
