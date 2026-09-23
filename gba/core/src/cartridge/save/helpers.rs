pub(crate) fn read_slice(slice: &[u8], off: usize, width: u8) -> u32 {
    match width {
        4 => {
            let b0 = *slice.get(off).unwrap_or(&0xFF) as u32;
            let b1 = *slice.get(off + 1).unwrap_or(&0xFF) as u32;
            let b2 = *slice.get(off + 2).unwrap_or(&0xFF) as u32;
            let b3 = *slice.get(off + 3).unwrap_or(&0xFF) as u32;
            b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
        }
        2 => {
            let b0 = *slice.get(off).unwrap_or(&0xFF) as u32;
            let b1 = *slice.get(off + 1).unwrap_or(&0xFF) as u32;
            b0 | (b1 << 8)
        }
        _ => *slice.get(off).unwrap_or(&0xFF) as u32,
    }
}

pub(crate) fn write_slice(slice: &mut [u8], off: usize, width: u8, value: u32) {
    match width {
        4 => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 1) {
                *b = ((value >> 8) & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 2) {
                *b = ((value >> 16) & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 3) {
                *b = ((value >> 24) & 0xFF) as u8;
            }
        }
        2 => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 1) {
                *b = ((value >> 8) & 0xFF) as u8;
            }
        }
        _ => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
        }
    }
}

/// Replicate an 8-bit Game Pak save-bus value across the requested CPU width.
pub(crate) fn repeat_byte(value: u8, width: u8) -> u32 {
    match width {
        4 => u32::from(value) * 0x01010101,
        2 => u32::from(value) * 0x0101,
        _ => u32::from(value),
    }
}

/// Select the byte lane driven by an unaligned 16/32-bit store to the 8-bit save bus.
pub(crate) fn selected_write_byte(address: u32, width: u8, value: u32) -> u8 {
    let lane = address & u32::from(width.saturating_sub(1));
    (value >> (lane * 8)) as u8
}

#[cfg(test)]
mod tests {
    use super::{repeat_byte, selected_write_byte};

    #[test]
    fn save_bus_repeats_byte_for_wide_reads() {
        assert_eq!(repeat_byte(0xA5, 1), 0xA5);
        assert_eq!(repeat_byte(0xA5, 2), 0xA5A5);
        assert_eq!(repeat_byte(0xA5, 4), 0xA5A5A5A5);
    }

    #[test]
    fn save_bus_selects_addressed_lane_for_wide_writes() {
        assert_eq!(selected_write_byte(0, 2, 0xAABB), 0xBB);
        assert_eq!(selected_write_byte(1, 2, 0xAABB), 0xAA);
        assert_eq!(selected_write_byte(0, 4, 0xAABBCCDD), 0xDD);
        assert_eq!(selected_write_byte(1, 4, 0xAABBCCDD), 0xCC);
        assert_eq!(selected_write_byte(2, 4, 0xAABBCCDD), 0xBB);
        assert_eq!(selected_write_byte(3, 4, 0xAABBCCDD), 0xAA);
    }
}
