use super::super::Register;
use super::{Accumulate, AccumulateMemory};

fn decrement(register: &mut Register, data: u8) -> u8 {
    let result = data.wrapping_sub(1);
    register.set_nz_from_value(result);
    result
}

accumulate!(Dex, Register::get_x, Register::set_x, decrement);
accumulate!(Dey, Register::get_y, Register::set_y, decrement);
accumulate_memory!(Dec, decrement);
