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
    if brightness == 0 {
        return 0;
    }
    let expanded = (u16::from(channel) << 3) | (u16::from(channel) >> 2);
    ((expanded * (u16::from(brightness) + 1) + 8) / 16) as u8
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
