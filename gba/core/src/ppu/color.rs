pub(crate) fn read_color(palette: &[u8], index: usize) -> u16 {
    let offset = index * 2;
    u16::from_le_bytes([palette[offset], palette[offset + 1]]) & 0x7FFF
}

/// BGR555(5bit/channel) -> RGBA8888 展開
/// GBAは15bitカラーだが、現代の8bit/channelへ展開する際は
/// 単純な `v<<3` (0-248) ではなく、ハードウェアのDAC特性に近い
/// `v*255/31` の近似である `(v<<3)|(v>>2)` (0-255) を用いる。
/// これは `v*255/31` と最大1の差で、mGBA等でも採用される。
/// 例: 31->255, 0->0, 4->33, 9->74, 6->49
/// 参照PNGが `v<<3` (例: 4->32)で生成されていても、15bitレベルでは同一なため
/// 検証側でBGR555に丸めて比較する (verify.rsのBGR555 tolerant)。
pub(crate) fn rgba8888(color: u16) -> u32 {
    let r = ((((color) & 0x1F) << 3) | (((color) & 0x1F) >> 2)) as u8;
    let g = ((((color >> 5) & 0x1F) << 3) | (((color >> 5) & 0x1F) >> 2)) as u8;
    let b = ((((color >> 10) & 0x1F) << 3) | (((color >> 10) & 0x1F) >> 2)) as u8;
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
