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
