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
    let mut x2 = usize::from(horizontal & 0xFF);
    let y1 = usize::from(vertical >> 8);
    let mut y2 = usize::from(vertical & 0xFF);
    // GBATEK Window: garbage X2>240 or X1>X2 is interpreted as X2=240;
    // garbage Y2>160 or Y1>Y2 is interpreted as Y2=160 (no wrap).
    if x2 > 240 || x1 > x2 {
        x2 = 240;
    }
    if y2 > 160 || y1 > y2 {
        y2 = 160;
    }
    (x1..x2).contains(&x) && (y1..y2).contains(&y)
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

    #[test]
    fn window_garbage_clamps_without_wrap() {
        // GBATEK: X1>X2 is garbage interpreted as X2=240 (no wrap to [0,X2)).
        // X1=200,X2=100 -> [200,240); x=50 must be outside.
        let horizontal = (200u16 << 8) | 100u16;
        let vertical = 160u16;
        assert!(in_window(horizontal, vertical, 210, 10));
        assert!(!in_window(horizontal, vertical, 50, 10));
        assert!(!in_window(horizontal, vertical, 210, 170));
    }
}
