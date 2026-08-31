/// Barrel shifter: LSL/LSR/ASR/ROR + RRX
/// shift_type: 0=LSL, 1=LSR, 2=ASR, 3=ROR
pub fn barrel_shift(rm: u32, shift_type: u8, amount: u32, carry_in: bool) -> (u32, bool) {
    let amount = amount & 0xFF;
    match shift_type & 0b11 {
        0b00 => {
            // LSL
            if amount == 0 {
                (rm, carry_in)
            } else if amount < 32 {
                let carry = (rm >> (32 - amount)) & 1 != 0;
                (rm << amount, carry)
            } else if amount == 32 {
                let carry = rm & 1 != 0;
                (0, carry)
            } else {
                (0, false)
            }
        }
        0b01 => {
            // LSR
            if amount == 0 || amount == 32 {
                let carry = (rm >> 31) & 1 != 0;
                (0, carry)
            } else if amount < 32 {
                let carry = (rm >> (amount - 1)) & 1 != 0;
                (rm >> amount, carry)
            } else {
                (0, false)
            }
        }
        0b10 => {
            // ASR
            if amount == 0 || amount >= 32 {
                let carry = (rm >> 31) & 1 != 0;
                let val = if (rm >> 31) & 1 != 0 { 0xFFFFFFFF } else { 0 };
                (val, carry)
            } else {
                let carry = (rm >> (amount - 1)) & 1 != 0;
                let val = ((rm as i32) >> amount) as u32;
                (val, carry)
            }
        }
        _ => {
            // ROR
            if amount == 0 {
                // RRX
                let carry = rm & 1 != 0;
                let val = (carry_in as u32) << 31 | (rm >> 1);
                (val, carry)
            } else {
                let rot = amount % 32;
                if rot == 0 {
                    let carry = (rm >> 31) & 1 != 0;
                    (rm, carry)
                } else {
                    let carry = (rm >> (rot - 1)) & 1 != 0;
                    (rm.rotate_right(rot), carry)
                }
            }
        }
    }
}

/// N/S/I サイクル算出ヘルパー — Cognitive Complexity 対策
pub fn calc_cycles(is_sequential: bool, is_internal: bool) -> u32 {
    if is_internal {
        1
    } else if is_sequential {
        1 // S
    } else {
        1 // N — Waitは bus.cycles_for で別途加算
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsl_shifts() {
        assert_eq!(barrel_shift(0x1, 0, 1, false).0, 0x2);
    }

    #[test]
    fn ror_rrx() {
        let (v, c) = barrel_shift(0x0000_0001, 3, 0, true);
        assert_eq!(v, 0x8000_0000);
        assert!(c);
    }

    #[test]
    fn lsr_zero_is_zero_with_carry() {
        let (v, c) = barrel_shift(0x8000_0000, 1, 0, false);
        assert_eq!(v, 0);
        assert!(c);
    }
}
