pub(crate) fn read_color(palette: &[u8], index: usize) -> u16 {
    let offset = index * 2;
    u16::from_le_bytes([palette[offset], palette[offset + 1]]) & 0x7FFF
}

/// BGR555(5bit/channel) -> RGBA8888 展開
/// GBAは15bitカラーだが、現代の8bit/channelへ展開する際は
/// 単純な `v<<3` (0-248) ではなく、ハードウェアのDAC特性に近い
/// `v*255/31` の近似である `(v<<3)|(v>>2)` (0-255) を用いる。
/// これは `v*255/31` と最大1の差で、mGBA等でも採用される。
/// Near "Color Emulation" の Color precision 章が求める bit-repeat 展開
/// (`rrr -> rrrrrrrr` 系)そのものでもあり、記事適合である。
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
    // NBA Merge: blend rounds to nearest (not truncation). The hardware
    // also keeps a 6th green bit through the blend; its exact source
    // (palette bit layout) is unconfirmed, so only rounding is modeled
    // here — recorded as residual P14 investigation.
    let blend = |shift: u32| {
        let a = u32::from((first >> shift) & 0x1F);
        let b = u32::from((second >> shift) & 0x1F);
        (((a * u32::from(eva) + b * u32::from(evb) + 8) >> 4).min(31) as u16) << shift
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

/// GBA LCD color emulation (presentation stage only).
///
/// Reference: Near "Color Emulation", LCD emulation: Game Boy Advance
/// (Talarubi's formula). The original AGB-001 LCD washes colors out, so
/// developers exaggerated palettes; shown raw on an sRGB monitor they look
/// oversaturated ("technicolor nightmare"). This filter reproduces the LCD
/// response: per-channel `pow(v/31, lcdGamma=4.0)`, a primaries cross-talk
/// matrix, then `pow(., 1/outGamma=2.2)`:
///
/// ```text
/// r = ((  0*lb +  50*lg + 255*lr) / 255) ^ (1/2.2)
/// g = (( 30*lb + 230*lg +  10*lr) / 255) ^ (1/2.2)
/// b = ((220*lb +  10*lg +  50*lr) / 255) ^ (1/2.2)
/// ```
///
/// The article scales by `(0xffff * 255 / 280)` for its 16-bit pipeline;
/// here the full-scale `0xffff` is replaced by `0xff` for 8-bit output,
/// keeping the article's `255/280` dimming headroom, then rounded+clamped.
///
/// This is intentionally *not* applied to the core framebuffer: the PPU
/// framebuffer stays a bit-exact BGR555 expansion (ROM tests verify
/// BGR555-exact pixels). Frontends apply this at presentation time, the
/// same layering as ares/mGBA color-correction shaders.
pub fn gba_lcd_rgba8888(color: u16) -> u32 {
    lcd_lut()[usize::from(color & 0x7FFF)]
}

/// Apply [`gba_lcd_rgba8888`] to a whole framebuffer in place.
///
/// The framebuffer stores the bit-repeat expansion (`rgba8888`), whose
/// upper 5 bits round-trip exactly (`(v << 3 | v >> 2) >> 3 == v`), so the
/// 5-bit source is recovered losslessly before filtering.
pub fn apply_gba_lcd_filter(frame: &mut [u32]) {
    for pixel in frame.iter_mut() {
        let bytes = pixel.to_le_bytes();
        let color = u16::from(bytes[0] >> 3)
            | (u16::from(bytes[1] >> 3) << 5)
            | (u16::from(bytes[2] >> 3) << 10);
        *pixel = gba_lcd_rgba8888(color);
    }
}

const LCD_GAMMA: f64 = 4.0;
const OUT_GAMMA: f64 = 2.2;
/// 8-bit full scale keeping the article's 255/280 dimming headroom.
const LCD_FULL_SCALE: f64 = 255.0 * 255.0 / 280.0;

fn lcd_channel(value: u16) -> f64 {
    (f64::from(value) / 31.0).powf(LCD_GAMMA)
}

fn lcd_output(linear: f64) -> u8 {
    (linear.powf(1.0 / OUT_GAMMA) * LCD_FULL_SCALE)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn lcd_correct(color: u16) -> u32 {
    let r = color & 0x1F;
    let g = (color >> 5) & 0x1F;
    let b = (color >> 10) & 0x1F;
    let lr = lcd_channel(r);
    let lg = lcd_channel(g);
    let lb = lcd_channel(b);
    let out_r = lcd_output((50.0 * lg + 255.0 * lr) / 255.0);
    let out_g = lcd_output((30.0 * lb + 230.0 * lg + 10.0 * lr) / 255.0);
    let out_b = lcd_output((220.0 * lb + 10.0 * lg + 50.0 * lr) / 255.0);
    u32::from_le_bytes([out_r, out_g, out_b, 0xFF])
}

static LCD_LUT: std::sync::OnceLock<[u32; 32768]> = std::sync::OnceLock::new();

fn lcd_lut() -> &'static [u32; 32768] {
    LCD_LUT.get_or_init(|| {
        let mut table = [0u32; 32768];
        for (index, entry) in table.iter_mut().enumerate() {
            *entry = lcd_correct(index as u16);
        }
        table
    })
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

    #[test]
    fn expansion_is_bit_repeat_per_article() {
        // Near "Color Emulation", Color precision: source bits repeat to
        // fill the target (`rrr -> rrrrrrrr`); for 5->8 bits that is
        // `v << 3 | v >> 2`, exactly `v * 255 / 31` rounded.
        for v in 0..32u16 {
            let expanded = (v << 3 | v >> 2) as u8;
            let exact = (u32::from(v) * 255 / 31) as u8;
            assert!((expanded as i16 - exact as i16).abs() <= 1, "v={v}");
            let c = v | (v << 5) | (v << 10);
            assert_eq!(
                rgba8888(c).to_le_bytes(),
                [expanded, expanded, expanded, 255]
            );
            // Round-trips: the LCD frame filter recovers 5-bit sources exactly.
            assert_eq!((expanded >> 3) as u16, v);
        }
    }

    #[test]
    fn lcd_black_stays_black() {
        assert_eq!(gba_lcd_rgba8888(0).to_le_bytes(), [0, 0, 0, 255]);
    }

    #[test]
    fn lcd_white_is_dimmed_near_white() {
        // Hand-derived from the article formula: white mixes to
        // r=(305/255)^(1/2.2)*232.05, g=(270/255)^.., b=(280/255)^...
        let [r, g, b, a] = gba_lcd_rgba8888(0x7FFF).to_le_bytes();
        assert_eq!(a, 255);
        assert_eq!((r, g, b), (252, 238, 242));
    }

    #[test]
    fn lcd_primaries_have_crosstalk() {
        // Pure red must excite green/blue via the primaries matrix,
        // with red still dominant.
        let [r, g, b, _] = gba_lcd_rgba8888(0x001F).to_le_bytes();
        assert!(r > g && r > b && g > 0 && b > 0, "got ({r},{g},{b})");
        assert_eq!((r, g, b), (232, 53, 111));
    }

    #[test]
    fn lcd_gray_ramp_is_monotonic() {
        let mut prev = [0u8; 3];
        for v in 0..32u16 {
            let c = v | (v << 5) | (v << 10);
            let [r, g, b, _] = gba_lcd_rgba8888(c).to_le_bytes();
            assert!(r >= prev[0] && g >= prev[1] && b >= prev[2], "v={v}");
            prev = [r, g, b];
        }
    }

    #[test]
    fn lcd_frame_filter_matches_single_pixel() {
        let mut frame = [rgba8888(0x03E0), rgba8888(0x7C00)];
        apply_gba_lcd_filter(&mut frame);
        assert_eq!(frame[0], gba_lcd_rgba8888(0x03E0));
        assert_eq!(frame[1], gba_lcd_rgba8888(0x7C00));
    }
}
