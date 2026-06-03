// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::{Core, Mode7Registers};

use super::{
    BgLayer, VISIBLE_BG_Y_OFFSET,
    color::{cgram_color_rgba, put_pixel},
    main_screen_for_line, presented_bg_line, use_presented_bg_scroll,
};

pub(super) fn render_mode7_bg1(
    core: &Core,
    brightness: u8,
    current_tm: u8,
    use_presented_tm: bool,
    rgba: &mut [u8],
) {
    let registers = core.mode7_registers();
    let current_hofs = i32::from(core.bg1_hofs());
    let current_vofs = i32::from(core.bg1_vofs());
    let use_presented_scroll = use_presented_bg_scroll(core, BgLayer::Bg1);

    for screen_y in 0..SCREEN_HEIGHT {
        if main_screen_for_line(core, screen_y, current_tm, use_presented_tm)
            & BgLayer::Bg1.tm_mask()
            == 0
        {
            continue;
        }
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, BgLayer::Bg1, screen_y))
            .flatten();
        let context = Mode7RenderContext {
            registers,
            hofs: presented.map_or(current_hofs, |line| i32::from(line.hofs)),
            vofs: presented.map_or(current_vofs, |line| i32::from(line.vofs)),
            brightness,
        };
        let mode7_screen_y = (screen_y + VISIBLE_BG_Y_OFFSET) as i32;
        for screen_x in 0..SCREEN_WIDTH {
            if let Some(color) = mode7_pixel(core, &context, screen_x as i32, mode7_screen_y) {
                put_pixel(rgba, screen_x, screen_y, color);
            }
        }
    }
}

fn mode7_pixel(
    core: &Core,
    context: &Mode7RenderContext,
    screen_x: i32,
    screen_y: i32,
) -> Option<[u8; 4]> {
    let (source_x, source_y) = mode7_source_coordinates(context, screen_x, screen_y);
    let color = mode7_vram_pixel(core, source_x, source_y);
    if color == 0 {
        return None;
    }

    Some(cgram_color_rgba(
        core,
        usize::from(color),
        context.brightness,
    ))
}

fn mode7_source_coordinates(
    context: &Mode7RenderContext,
    screen_x: i32,
    screen_y: i32,
) -> (usize, usize) {
    let registers = context.registers;
    let center_x = i32::from(registers.x);
    let center_y = i32::from(registers.y);
    let source_x = screen_x + context.hofs - center_x;
    let source_y = screen_y + context.vofs - center_y;
    let transformed_x =
        (i32::from(registers.a) * source_x + i32::from(registers.b) * source_y) / 256 + center_x;
    let transformed_y =
        (i32::from(registers.c) * source_x + i32::from(registers.d) * source_y) / 256 + center_y;

    (
        transformed_x.rem_euclid(1024) as usize,
        transformed_y.rem_euclid(1024) as usize,
    )
}

fn mode7_vram_pixel(core: &Core, source_x: usize, source_y: usize) -> u8 {
    let tile_x = (source_x / 8) & 0x7F;
    let tile_y = (source_y / 8) & 0x7F;
    let pixel_x = source_x & 0x07;
    let pixel_y = source_y & 0x07;
    let tile_number = usize::from(core.peek_vram((tile_y * 128 + tile_x) * 2));
    core.peek_vram((tile_number * 64 + pixel_y * 8 + pixel_x) * 2 + 1)
}

#[derive(Debug, Clone, Copy)]
struct Mode7RenderContext {
    registers: Mode7Registers,
    hofs: i32,
    vofs: i32,
    brightness: u8,
}

#[cfg(test)]
mod tests {
    use super::{Mode7RenderContext, mode7_source_coordinates};
    use nerust_snes_core::Mode7Registers;

    #[test]
    fn mode7_source_coordinates_apply_identity_scale_and_wrapping() {
        let mut context = Mode7RenderContext {
            registers: Mode7Registers {
                a: 0x0100,
                d: 0x0100,
                ..Mode7Registers::default()
            },
            hofs: 0,
            vofs: 0,
            brightness: 0x0F,
        };

        assert_eq!(mode7_source_coordinates(&context, 3, 4), (3, 4));

        context.registers.a = 0x0200;
        context.registers.d = 0x0200;
        assert_eq!(mode7_source_coordinates(&context, 3, 4), (6, 8));

        context.hofs = -1;
        context.vofs = -2;
        assert_eq!(mode7_source_coordinates(&context, 0, 0), (1022, 1020));
    }
}
