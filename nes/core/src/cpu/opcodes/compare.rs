use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepStateEnum, Ppu, Register},
    exit_opcode,
};

pub(crate) trait Compare {
    fn comparer(register: &Register) -> u8;

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let a = Self::comparer(&core.register);
                let b = core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );

                core.register.set_nz_from_value(a.wrapping_sub(b));
                core.register.set_c(a >= b);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! compare {
    ($name:ident, $comparer:expr) => {
        pub(crate) struct $name;

        impl Compare for $name {
            fn comparer(register: &Register) -> u8 {
                $comparer(register)
            }
        }

        cpu_step_state_impl!($name);
    };
}

compare!(Cmp, Register::get_a);
compare!(Cpx, Register::get_x);
compare!(Cpy, Register::get_y);
