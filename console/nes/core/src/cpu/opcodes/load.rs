use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepStateEnum, Ppu, Register},
    exit_opcode,
};

pub(crate) trait Load {
    fn setter(register: &mut Register, value: u8);

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let a = core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );

                core.register.set_nz_from_value(a);
                Self::setter(&mut core.register, a);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

macro_rules! load {
    ($name:ident, $func:expr) => {
        pub(crate) struct $name;

        impl Load for $name {
            fn setter(register: &mut Register, value: u8) {
                ($func)(register, value);
            }
        }

        cpu_step_state_impl!($name);
    };
}

load!(Lda, Register::set_a);
load!(Ldx, Register::set_x);
load!(Ldy, Register::set_y);
