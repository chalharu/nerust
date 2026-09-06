pub(crate) fn read_color(palette: &[u8], index: usize) -> u16 {
    let offset = index * 2;
    u16::from_le_bytes([palette[offset], palette[offset + 1]]) & 0x7FFF
}

pub(crate) fn rgba8888(color: u16) -> u32 {
    let r = ((color & 0x1F) * 255 / 31) as u8;
    let g = (((color >> 5) & 0x1F) * 255 / 31) as u8;
    let b = (((color >> 10) & 0x1F) * 255 / 31) as u8;
    u32::from_le_bytes([r, g, b, 0xFF])
}

pub(crate) fn alpha_blend(first: u16, second: u16, eva: u8, evb: u8) -> u16 {
    let blend = |shift: u32| {
        let a = u32::from((first >> shift) & 0x1F);
        let b = u32::from((second >> shift) & 0x1F);
        (((a * u32::from(eva) + b * u32::from(evb)) >> 4).min(31) as u16) << shift
    };
    blend(0) | blend(5) | blend(10)
}

pub(crate) fn brighten(color: u16, amount: u8) -> u16 {
    change_brightness(color, amount, true)
}

pub(crate) fn darken(color: u16, amount: u8) -> u16 {
    change_brightness(color, amount, false)
}

fn change_brightness(color: u16, amount: u8, brighter: bool) -> u16 {
    let adjust = |shift: u32| {
        let value = u32::from((color >> shift) & 0x1F);
        let result = if brighter {
            value + (((31 - value) * u32::from(amount)) >> 4)
        } else {
            value - ((value * u32::from(amount)) >> 4)
        };
        (result.min(31) as u16) << shift
    };
    adjust(0) | adjust(5) | adjust(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_clamps_colors() {
        assert_eq!(rgba8888(0x001F).to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(alpha_blend(0x001F, 0x001F, 16, 16), 0x001F);
        assert_eq!(brighten(0, 16), 0x7FFF);
        assert_eq!(darken(0x7FFF, 16), 0);
    }
}
