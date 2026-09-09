/// BG mosaic: compress (x,y) to the origin of its mosaic block.
///
/// `mosaic` is the line-latched MOSAIC register (sampled at line start, see
/// `GbaPpu::capture_line_latch`): mid-scanline writes take effect on the next
/// line. HBlank/VBlank writes (IRQ, HBlank DMA, `sprite-hmosaic` style) still
/// apply to the next line exactly as before.
///
/// The compressed screen coordinate is used *before* the scroll offset is
/// added (`(x - x % h) + hofs` in `bg.rs`), i.e. mosaic is screen-fixed.
/// This matches hardware: horizontal mosaic is a post-process output latch on
/// screen pixels, and vertical mosaic holds the rendered source line
/// (RadDad772 "Notes on GBA PPU: How mosaic works"; Tonc gfx.htm: the
/// top-left pixel of each block fills the block). A scroll-inclusive variant
/// (`(x + hofs) - (x + hofs) % h`) would shift mosaic blocks with scrolling
/// and does not match hardware for the static-scroll case.
pub fn bg_mosaic(mosaic: u16, cnt: u16, x: usize, y: usize) -> (usize, usize) {
    if cnt & (1 << 6) == 0 {
        return (x, y);
    }
    let h = usize::from(mosaic & 0xF) + 1;
    let v = usize::from((mosaic >> 4) & 0xF) + 1;
    (x - x % h, y - y % v)
}

/// OBJ mosaic: the held *screen* pixel feeds the sprite (output-latch model,
/// like BG mosaic; mGBA `SPRITE_MOSAIC_LOOP` phases by output `outX % mosaicH`,
/// not by sprite-local position). A mosaic block starting off-sprite clamps
/// to the sprite edge pixel.
pub fn apply_obj_mosaic(
    mosaic: u16,
    screen: (usize, usize),
    origin: (i32, i32),
    local: &mut (i32, i32),
    field: (usize, usize),
) {
    let h = usize::from((mosaic >> 8) & 0xF) + 1;
    let v = usize::from((mosaic >> 12) & 0xF) + 1;
    let held_x = (screen.0 - screen.0 % h) as i32;
    let held_y = (screen.1 - screen.1 % v) as i32;
    local.0 = (held_x - origin.0).clamp(0, field.0 as i32 - 1);
    local.1 = (held_y - origin.1).clamp(0, field.1 as i32 - 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mosaic_expands() {
        assert_eq!(bg_mosaic(0x11, 1 << 6, 5, 7), (4, 6));
        // Screen-anchored: sprite at origin (1, 1), screen (5, 7), mosaic 2x2
        // holds screen (4, 6) -> local (3, 5).
        let mut local = (4, 6);
        apply_obj_mosaic(0x1100, (5, 7), (1, 1), &mut local, (8, 8));
        assert_eq!(local, (3, 5));
    }
}
