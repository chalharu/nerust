use nerust_snes_core::Core;

pub(super) fn cgram_color_rgba(core: &Core, color_index: usize, brightness: u8) -> [u8; 4] {
    let base = color_index * 2;
    let color = u16::from_le_bytes([core.peek_cgram(base), core.peek_cgram(base + 1)]) & 0x7FFF;
    snes_color_to_rgba(color, brightness)
}

const CHANNEL_TO_8BIT: [u8; 32] = {
    let mut lut = [0u8; 32];
    let mut c = 0u32;
    loop {
        lut[c as usize] = ((c * 255 + 15) / 31) as u8;
        c += 1;
        if c == 32 {
            break;
        }
    }
    lut
};

pub(super) fn snes_color_to_rgba(color: u16, brightness: u8) -> [u8; 4] {
    let red = scale_channel((color & 0x1F) as u8, brightness);
    let green = scale_channel(((color >> 5) & 0x1F) as u8, brightness);
    let blue = scale_channel(((color >> 10) & 0x1F) as u8, brightness);
    [red, green, blue, 0xFF]
}

fn scale_channel(channel: u8, brightness: u8) -> u8 {
    if brightness == 0 {
        return 0;
    }
    let expanded = u16::from(CHANNEL_TO_8BIT[channel as usize]);
    ((expanded * (u16::from(brightness) + 1) + 8) / 16) as u8
}

pub(super) fn cgram_raw_color(core: &Core, color_index: usize) -> u16 {
    let base = color_index * 2;
    u16::from_le_bytes([core.peek_cgram(base), core.peek_cgram(base + 1)]) & 0x7FFF
}

pub(super) fn apply_color_math(main_15: u16, sub_15: u16, subtract: bool, half: bool) -> u16 {
    let mr = (main_15 >> 0) & 0x1F;
    let mg = (main_15 >> 5) & 0x1F;
    let mb = (main_15 >> 10) & 0x1F;
    let sr = (sub_15 >> 0) & 0x1F;
    let sg = (sub_15 >> 5) & 0x1F;
    let sb = (sub_15 >> 10) & 0x1F;

    let mut r = if subtract {
        mr.saturating_sub(sr)
    } else {
        (mr + sr).min(31)
    };
    let mut g = if subtract {
        mg.saturating_sub(sg)
    } else {
        (mg + sg).min(31)
    };
    let mut b = if subtract {
        mb.saturating_sub(sb)
    } else {
        (mb + sb).min(31)
    };

    if half {
        r = (r + 1) >> 1;
        g = (g + 1) >> 1;
        b = (b + 1) >> 1;
    }

    r | (g << 5) | (b << 10)
}

pub(super) fn put_pixel(rgba: &mut [u8], width: usize, x: usize, y: usize, color: [u8; 4]) {
    let offset = (y * width + x) * 4;
    rgba[offset..offset + 4].copy_from_slice(&color);
}

pub(super) fn opaque_black_screen(width: usize, height: usize) -> Vec<u8> {
    let mut rgba = vec![0; width * height * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::scale_channel;

    #[test]
    fn brightness_scaling_reaches_black_and_full_intensity() {
        assert_eq!(scale_channel(0x1F, 0x00), 0x00);
        assert_eq!(scale_channel(0x1F, 0x0F), 0xFF);
    }

    #[test]
    fn bit_replication_maps_5bit_to_8bit_correctly() {
        assert_eq!(scale_channel(0x00, 0x0F), 0x00);
        assert_eq!(scale_channel(0x1F, 0x0F), 0xFF);
        assert_eq!(scale_channel(0x10, 0x0F), 132); // (16<<3)|(16>>2) = 128+4 = 132
    }
}
