use super::{
    super::{Apu, Controller, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_opcode,
};

pub(crate) struct Bit;

impl CpuStepState for Bit {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
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
                let a = data & core.register.get_a();
                core.register.set_v(data & 0x40 != 0);
                core.register.set_z_from_value(a);
                core.register.set_n_from_value(data);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
