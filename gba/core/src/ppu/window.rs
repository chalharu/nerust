use super::PpuRegisters;
use crate::ppu::obj;

pub fn window_mask(
    registers: &PpuRegisters,
    x: usize,
    y: usize,
    vram: &[u8],
    palette: &[u8],
    oam: &[u8],
) -> u8 {
    let enabled = (registers.dispcnt >> 13) & 7;
    if enabled == 0 {
        return 0x3F;
    }
    if enabled & 1 != 0 && in_window(registers.winh[0], registers.winv[0], x, y) {
        return registers.winin as u8 & 0x3F;
    }
    if enabled & 2 != 0 && in_window(registers.winh[1], registers.winv[1], x, y) {
        return (registers.winin >> 8) as u8 & 0x3F;
    }
    if enabled & 4 != 0 && obj::pixel(registers, vram, palette, oam, x, y, true).is_some() {
        return (registers.winout >> 8) as u8 & 0x3F;
    }
    registers.winout as u8 & 0x3F
}

fn in_window(horizontal: u16, vertical: u16, x: usize, y: usize) -> bool {
    let x1 = usize::from(horizontal >> 8);
    let x2 = usize::from(horizontal & 0xFF);
    let y1 = usize::from(vertical >> 8);
    let y2 = usize::from(vertical & 0xFF);
    range_contains(x1, x2, x) && range_contains(y1, y2, y)
}

fn range_contains(start: usize, end: usize, value: usize) -> bool {
    if start <= end {
        (start..end).contains(&value)
    } else {
        value >= start || value < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_contains_wrapping() {
        // 20..0 wraps: 0..20 is not inside 20..0, 30 is inside
        assert!(in_window(0x1400, 0x1400, 30, 30));
        assert!(!in_window(0x1400, 0x1400, 10, 10));
        assert!(in_window(0x0014, 0x0014, 10, 10));
        assert!(!in_window(0x0014, 0x0014, 30, 30));
    }
}
