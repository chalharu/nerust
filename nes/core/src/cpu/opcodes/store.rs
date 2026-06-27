use super::{
    super::{Apu, Controller, Core, CpuCartridgeBus, CpuStepStateEnum, Ppu, Register},
    exit_opcode,
};

pub(crate) trait Store {
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
                let data = Self::getter(&core.register);
                core.memory.write(
                    core.internal_stat.get_address(),
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

macro_rules! store {
    ($name:ident, $func:expr) => {
        pub(crate) struct $name;

        impl Store for $name {
            fn getter(register: &Register) -> u8 {
                $func(register)
            }
        }

        cpu_step_state_impl!($name);
    };
}

store!(Sta, Register::get_a);
store!(Stx, Register::get_x);
store!(Sty, Register::get_y);
