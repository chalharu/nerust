/// BG mosaic: compress (x,y) to the origin of its mosaic block.
pub fn bg_mosaic(
    registers: &crate::ppu::PpuRegisters,
    cnt: u16,
    x: usize,
    y: usize,
) -> (usize, usize) {
    if cnt & (1 << 6) == 0 {
        return (x, y);
    }
    let h = usize::from(registers.mosaic & 0xF) + 1;
    let v = usize::from((registers.mosaic >> 4) & 0xF) + 1;
    (x - x % h, y - y % v)
}

/// OBJ mosaic: compress local coordinates inside a sprite.
pub fn apply_obj_mosaic(mosaic: u16, x: &mut i32, y: &mut i32) {
    let h = i32::from((mosaic >> 8) & 0xF) + 1;
    let v = i32::from((mosaic >> 12) & 0xF) + 1;
    *x -= *x % h;
    *y -= *y % v;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ppu::PpuRegisters;

    #[test]
    fn mosaic_expands() {
        let regs = PpuRegisters {
            mosaic: 0x11,
            ..Default::default()
        };
        assert_eq!(bg_mosaic(&regs, 1 << 6, 5, 7), (4, 6));
        let mut x = 5;
        let mut y = 7;
        let regs = PpuRegisters {
            mosaic: 0x1100,
            ..Default::default()
        };
        apply_obj_mosaic(regs.mosaic, &mut x, &mut y);
        assert_eq!((x, y), (4, 6));
    }
}
