use nerust_snes_core::Core;

use super::{
    BgLayer,
    color::{cgram_color_rgba, put_pixel},
    main_screen_for_line, presented_bg_line, use_presented_bg_scroll,
};

pub(super) fn render_mode7_bg1(
    core: &Core,
    brightness: u8,
    current_tm: u8,
    use_presented_tm: bool,
    interlace_enabled: bool,
    interlace_field: bool,
    render_width: usize,
    render_height: usize,
    rgba: &mut [u8],
) {
    let registers = core.mode7_registers();
    let a = i32::from(registers.a);
    let b = i32::from(registers.b);
    let c = i32::from(registers.c);
    let d = i32::from(registers.d);
    let center_x = i32::from(registers.x);
    let center_y = i32::from(registers.y);
    let m7sel = registers.m7sel;
    let repeat = m7sel & 0x03;
    let extbg = m7sel & 0x80 != 0;
    let raw_hofs = i32::from(core.bg1_hofs()) & 0x3FF;
    let raw_vofs = i32::from(core.bg1_vofs()) & 0x3FF;
    // Mode 7 scroll offsets are 10-bit signed values.
    // Sign-extend from 10 bits to match bsnes behavior.
    let current_hofs = if raw_hofs & 0x200 != 0 {
        raw_hofs | !0x3FF
    } else {
        raw_hofs
    };
    let current_vofs = if raw_vofs & 0x200 != 0 {
        raw_vofs | !0x3FF
    } else {
        raw_vofs
    };
    let use_presented_scroll = use_presented_bg_scroll(core, BgLayer::Bg1);

    for screen_y in 0..render_height {
        if main_screen_for_line(core, screen_y, render_height, current_tm, use_presented_tm)
            & BgLayer::Bg1.tm_mask()
            == 0
        {
            continue;
        }
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, BgLayer::Bg1, screen_y, render_height))
            .flatten();
        let raw_vofs = presented.map_or(current_vofs, |line| {
            let raw = i32::from(line.vofs) & 0x3FF;
            if raw & 0x200 != 0 { raw | !0x3FF } else { raw }
        });
        let effective_vofs = if interlace_enabled && interlace_field {
            (raw_vofs & !1) | 1
        } else if interlace_enabled {
            raw_vofs & !1
        } else {
            raw_vofs
        };
        let hofs = presented.map_or(current_hofs, |line| {
            let raw = i32::from(line.hofs) & 0x3FF;
            if raw & 0x200 != 0 { raw | !0x3FF } else { raw }
        });
        let vofs = effective_vofs;

        // bsnes-style two-step Mode 7 coordinate computation:
        // 1. Per-scanline origin (with 6-bit sub-pixel truncation)
        // 2. Per-pixel contribution
        let dx = hofs - center_x;
        let dy = vofs - center_y;
        let mode7_screen_y = screen_y as i32 + 1;
        let origin_x =
            ((a * dx) & !63) + ((b * dy) & !63) + ((b * mode7_screen_y) & !63) + (center_x << 8);
        let origin_y =
            ((c * dx) & !63) + ((d * dy) & !63) + ((d * mode7_screen_y) & !63) + (center_y << 8);

        for screen_x in 0..render_width {
            let mode7_screen_x = screen_x as i32;
            let transformed_x = (origin_x + a * mode7_screen_x) >> 8;
            let transformed_y = (origin_y + c * mode7_screen_x) >> 8;

            let palette = mode7_pixel_value(core, transformed_x, transformed_y, repeat);
            // In EXTBG mode, BG1 pixels with bit 5 set are handled by
            // BG2 instead (BG2 overlay pass follows this one).
            if extbg && palette & 0x20 != 0 {
                continue;
            }
            let color = cgram_color_rgba(core, usize::from(palette), brightness);
            put_pixel(rgba, render_width, screen_x, screen_y, color);
        }
    }

    // In EXTBG mode, BG2 overlays on top of BG1 using the same Mode 7
    // coordinates but interpreting the pixel byte differently:
    //   bits 0-6 = BG2 color index (0-127)
    //   bit 7    = BG2 priority
    // When BG2 color index is 0, the pixel is transparent and BG1 shows through.
    if extbg {
        render_mode7_bg2_overlay(
            core,
            brightness,
            a,
            b,
            c,
            d,
            center_x,
            center_y,
            repeat,
            current_hofs,
            current_vofs,
            use_presented_scroll,
            interlace_enabled,
            interlace_field,
            render_width,
            render_height,
            rgba,
        );
    }
}

fn render_mode7_bg2_overlay(
    core: &Core,
    brightness: u8,
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    center_x: i32,
    center_y: i32,
    repeat: u8,
    current_hofs: i32,
    current_vofs: i32,
    use_presented_scroll: bool,
    interlace_enabled: bool,
    interlace_field: bool,
    render_width: usize,
    render_height: usize,
    rgba: &mut [u8],
) {
    for screen_y in 0..render_height {
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, BgLayer::Bg1, screen_y, render_height))
            .flatten();
        let raw_vofs = presented.map_or(current_vofs, |line| {
            let raw = i32::from(line.vofs) & 0x3FF;
            if raw & 0x200 != 0 { raw | !0x3FF } else { raw }
        });
        let effective_vofs = if interlace_enabled && interlace_field {
            (raw_vofs & !1) | 1
        } else if interlace_enabled {
            raw_vofs & !1
        } else {
            raw_vofs
        };
        let hofs = presented.map_or(current_hofs, |line| {
            let raw = i32::from(line.hofs) & 0x3FF;
            if raw & 0x200 != 0 { raw | !0x3FF } else { raw }
        });
        let vofs = effective_vofs;

        let dx = hofs - center_x;
        let dy = vofs - center_y;
        let mode7_screen_y = screen_y as i32 + 1;
        let origin_x =
            ((a * dx) & !63) + ((b * dy) & !63) + ((b * mode7_screen_y) & !63) + (center_x << 8);
        let origin_y =
            ((c * dx) & !63) + ((d * dy) & !63) + ((d * mode7_screen_y) & !63) + (center_y << 8);

        for screen_x in 0..render_width {
            let transformed_x = (origin_x + a * screen_x as i32) >> 8;
            let transformed_y = (origin_y + c * screen_x as i32) >> 8;

            // BG2 pixel interpretation (per Fullsnes EXTBG):
            // bits 0-2: BG1 color index (already handled in BG1 pass)
            // bit 5: BG2 enable (1 = force BG2 overlay)
            // bits 0-6: BG2 color index (when BG2 is enabled)
            let raw = mode7_pixel_value(core, transformed_x, transformed_y, repeat);
            if raw & 0x20 == 0 {
                continue;
            }
            let bg2_color = raw & 0x7F;
            if bg2_color == 0 {
                continue;
            }
            let color = cgram_color_rgba(core, usize::from(bg2_color), brightness);
            put_pixel(rgba, render_width, screen_x, screen_y, color);
        }
    }
}

pub(super) fn mode7_pixel_value(
    core: &Core,
    transformed_x: i32,
    transformed_y: i32,
    repeat: u8,
) -> u8 {
    let out_of_bounds_mask: i32 = !1023;
    let out_of_bounds = (transformed_x | transformed_y) & out_of_bounds_mask != 0;

    let tile_x = ((transformed_x >> 3) as u32 & 0x7F) as usize;
    let tile_y = ((transformed_y >> 3) as u32 & 0x7F) as usize;
    let pixel_x = (transformed_x as u32 & 0x07) as usize;
    let pixel_y = (transformed_y as u32 & 0x07) as usize;

    let tile_number = if repeat == 3 && out_of_bounds {
        0
    } else {
        usize::from(core.peek_vram((tile_y * 128 + tile_x) * 2))
    };

    if repeat == 2 && out_of_bounds {
        0
    } else {
        core.peek_vram((tile_number * 64 + pixel_y * 8 + pixel_x) * 2 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::mode7_pixel_value;
    use nerust_snes_core::{Core, CpuState};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn build_lorom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST SCREEN ROM      ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&reset_vector.to_le_bytes());
        rom
    }

    fn run_until_stopped(core: &mut Core, max_steps: usize) {
        for _ in 0..max_steps {
            core.step().unwrap();
            if core.current_state() == CpuState::Stopped {
                return;
            }
        }

        panic!("core did not stop within {max_steps} steps");
    }

    #[test]
    fn mode7_pixel_value_reads_bg_map_from_even_bytes_and_tiles_from_odd_bytes() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA9, 0x80, 0x8D, 0x00, 0x21, 0xA9, 0x80, 0x8D,
            0x15, 0x21, 0x9C, 0x16, 0x21, 0x9C, 0x17, 0x21, 0xA9, 0x00, 0x8D, 0x18, 0x21, 0xA9,
            0x5A, 0x8D, 0x19, 0x21, 0xDB,
        ];
        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 256);

        assert_eq!(core.peek_vram(0), 0x00);
        assert_eq!(core.peek_vram(1), 0x5A);
        assert_eq!(mode7_pixel_value(&core, 0, 0, 0), 0x5A);
    }
}
