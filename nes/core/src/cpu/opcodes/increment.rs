use super::{super::Register, Accumulate, AccumulateMemory};

fn increment(register: &mut Register, data: u8) -> u8 {
    let result = data.wrapping_add(1);
    register.set_nz_from_value(result);
    result
}

accumulate!(Inx, Register::get_x, Register::set_x, increment);
accumulate!(Iny, Register::get_y, Register::set_y, increment);
accumulate_memory!(Inc, increment);
