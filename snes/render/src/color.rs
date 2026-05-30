// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::Core;

pub(super) fn cgram_color_rgba(core: &Core, color_index: usize, brightness: u8) -> [u8; 4] {
    let base = color_index * 2;
    let color = u16::from_le_bytes([core.peek_cgram(base), core.peek_cgram(base + 1)]) & 0x7FFF;
    snes_color_to_rgba(color, brightness)
}

pub(super) fn snes_color_to_rgba(color: u16, brightness: u8) -> [u8; 4] {
    let red = scale_channel((color & 0x1F) as u8, brightness);
    let green = scale_channel(((color >> 5) & 0x1F) as u8, brightness);
    let blue = scale_channel(((color >> 10) & 0x1F) as u8, brightness);
    [red, green, blue, 0xFF]
}

fn scale_channel(channel: u8, brightness: u8) -> u8 {
    let expanded = (u16::from(channel) * 255 + 15) / 31;
    ((expanded * u16::from(brightness) + 7) / 15) as u8
}

pub(super) fn put_pixel(rgba: &mut [u8], x: usize, y: usize, color: [u8; 4]) {
    let offset = (y * SCREEN_WIDTH + x) * 4;
    rgba[offset..offset + 4].copy_from_slice(&color);
}

pub(super) fn opaque_black_screen() -> Vec<u8> {
    let mut rgba = vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::{opaque_black_screen, scale_channel};

    #[test]
    fn brightness_scaling_reaches_black_and_full_intensity() {
        assert_eq!(scale_channel(0x1F, 0x00), 0x00);
        assert_eq!(scale_channel(0x1F, 0x0F), 0xFF);
    }

    #[test]
    fn opaque_black_screen_uses_opaque_pixels() {
        let rgba = opaque_black_screen();

        assert_eq!(&rgba[..4], &[0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(&rgba[rgba.len() - 4..], &[0x00, 0x00, 0x00, 0xFF]);
    }
}
