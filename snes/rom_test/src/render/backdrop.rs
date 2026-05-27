// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::media::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::{Core, PresentedBackdropLine};

use super::color::{cgram_color_rgba, opaque_black_screen, put_pixel, snes_color_to_rgba};

pub(super) fn render_presented_backdrop(core: &Core) -> Vec<u8> {
    let fallback = current_backdrop_rgba(core);
    let mut rgba = opaque_black_screen();

    for screen_y in 0..SCREEN_HEIGHT {
        let line_color = core
            .presented_backdrop_line(screen_y)
            .map_or(fallback, presented_backdrop_line_rgba);
        for screen_x in 0..SCREEN_WIDTH {
            put_pixel(&mut rgba, screen_x, screen_y, line_color);
        }
    }

    rgba
}

fn current_backdrop_rgba(core: &Core) -> [u8; 4] {
    let inidisp = core.peek(0x002100);
    let brightness = inidisp & 0x0F;
    if inidisp & 0x80 != 0 || brightness == 0 {
        [0x00, 0x00, 0x00, 0xFF]
    } else {
        cgram_color_rgba(core, 0, brightness)
    }
}

fn presented_backdrop_line_rgba(line: PresentedBackdropLine) -> [u8; 4] {
    let brightness = line.inidisp & 0x0F;
    if line.inidisp & 0x80 != 0 || brightness == 0 {
        [0x00, 0x00, 0x00, 0xFF]
    } else {
        snes_color_to_rgba(line.color0, brightness)
    }
}
