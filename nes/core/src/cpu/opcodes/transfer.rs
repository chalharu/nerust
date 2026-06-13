use super::super::Register;
use super::Accumulate;

fn set_nz_from_value(register: &mut Register, data: u8) -> u8 {
    register.set_nz_from_value(data);
    data
}

accumulate!(Tax, Register::get_a, Register::set_x, set_nz_from_value);
accumulate!(Tay, Register::get_a, Register::set_y, set_nz_from_value);
accumulate!(Tsx, Register::get_sp, Register::set_x, set_nz_from_value);
accumulate!(Txa, Register::get_x, Register::set_a, set_nz_from_value);
accumulate!(Tya, Register::get_y, Register::set_a, set_nz_from_value);
accumulate!(Txs, Register::get_x, Register::set_sp, |_, v| v);
