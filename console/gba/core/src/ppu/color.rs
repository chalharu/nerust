pub(crate) fn read_color(palette: &[u8], index: usize) -> u16 {
    let offset = index * 2;
    u16::from_le_bytes([palette[offset], palette[offset + 1]]) & 0x7FFF
}

/// Expand BGR555 to RGBA8888 with bit-repeat (`v<<3|v>>2`), close to `v*255/31`.
pub(crate) fn rgba8888(color: u16) -> u32 {
    RGBA_LUT[usize::from(color & 0x7FFF)]
}

const fn expand_channel(value: u16) -> u8 {
    (((value & 0x1F) << 3) | ((value & 0x1F) >> 2)) as u8
}

const fn build_rgba_lut() -> [u32; 32768] {
    let mut table = [0u32; 32768];
    let mut color = 0usize;
    while color < 32768 {
        let c = color as u16;
        let r = expand_channel(c);
        let g = expand_channel(c >> 5);
        let b = expand_channel(c >> 10);
        table[color] = u32::from_le_bytes([r, g, b, 0xFF]);
        color += 1;
    }
    table
}

/// Bit-repeat expansion is a pure function of 15 bits: a compile-time
/// table replaces ~15 ALU ops per pixel with one load (bit-exact: the
/// same formula, evaluated at compile time).
static RGBA_LUT: [u32; 32768] = build_rgba_lut();

/// Reference direct-form blend (production uses [`BlendCache`];
/// kept under test-gate as the differential oracle).
#[cfg(test)]
pub(crate) fn alpha_blend(first: u16, second: u16, eva: u8, evb: u8) -> u16 {
    // Blend rounds to nearest (not truncation). The hardware
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

/// Per-channel blend of two 5-bit values (same formula as
/// `alpha_blend`'s inner closure, factored for tabulation).
const fn blend_channel(a: u16, b: u16, eva: u8, evb: u8) -> u16 {
    let v = (a as u32 * eva as u32 + b as u32 * evb as u32 + 8) >> 4;
    (if v > 31 { 31 } else { v }) as u16
}

/// Cached blend tables: the per-(factor) tables below rebuild when
/// their key changes (factor writes are rare; pixels in between hit
/// L1-cached loads instead of multiplies). Bit-exact: tables evaluate
/// the same formulas as the direct functions.
#[derive(Clone, Debug)]
pub(crate) struct BlendCache {
    alpha_key: (u8, u8),
    alpha: [u16; 1024],
    bright_key: u8,
    bright: [u16; 32],
    dark_key: u8,
    dark: [u16; 32],
}

impl BlendCache {
    pub(crate) fn new() -> Self {
        Self {
            alpha_key: (0xFF, 0xFF),
            alpha: [0; 1024],
            bright_key: 0xFF,
            bright: [0; 32],
            dark_key: 0xFF,
            dark: [0; 32],
        }
    }

    fn rebuild_alpha(&mut self, eva: u8, evb: u8) {
        for a in 0..32u16 {
            for b in 0..32u16 {
                self.alpha[(a as usize) << 5 | b as usize] = blend_channel(a, b, eva, evb);
            }
        }
        self.alpha_key = (eva, evb);
    }

    fn rebuild_bright(&mut self, amount: u8) {
        for v in 0..32u16 {
            self.bright[v as usize] = (v + (((31 - v) * u16::from(amount)) >> 4)).min(31);
        }
        self.bright_key = amount;
    }

    fn rebuild_dark(&mut self, amount: u8) {
        for v in 0..32u16 {
            self.dark[v as usize] = v - ((v * u16::from(amount)) >> 4);
        }
        self.dark_key = amount;
    }

    pub(crate) fn alpha_blend(&mut self, first: u16, second: u16, eva: u8, evb: u8) -> u16 {
        if self.alpha_key != (eva, evb) {
            self.rebuild_alpha(eva, evb);
        }
        let table = &self.alpha;
        let r = table[((first & 0x1F) << 5 | (second & 0x1F)) as usize];
        let g = table[(((first >> 5) & 0x1F) << 5 | ((second >> 5) & 0x1F)) as usize];
        let b = table[(((first >> 10) & 0x1F) << 5 | ((second >> 10) & 0x1F)) as usize];
        r | g << 5 | b << 10
    }

    pub(crate) fn brighten(&mut self, color: u16, amount: u8) -> u16 {
        if self.bright_key != amount {
            self.rebuild_bright(amount);
        }
        let table = &self.bright;
        table[(color & 0x1F) as usize]
            | table[((color >> 5) & 0x1F) as usize] << 5
            | table[((color >> 10) & 0x1F) as usize] << 10
    }

    pub(crate) fn darken(&mut self, color: u16, amount: u8) -> u16 {
        if self.dark_key != amount {
            self.rebuild_dark(amount);
        }
        let table = &self.dark;
        table[(color & 0x1F) as usize]
            | table[((color >> 5) & 0x1F) as usize] << 5
            | table[((color >> 10) & 0x1F) as usize] << 10
    }
}

/// Reference direct-form brightness (production uses [`BlendCache`];
/// kept under test-gate as the differential oracle).
#[cfg(test)]
pub(crate) fn brighten(color: u16, amount: u8) -> u16 {
    change_brightness(color, amount, true)
}

/// Reference direct-form brightness (production uses [`BlendCache`];
/// kept under test-gate as the differential oracle).
#[cfg(test)]
pub(crate) fn darken(color: u16, amount: u8) -> u16 {
    change_brightness(color, amount, false)
}

#[cfg(test)]
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

/// GBA LCD color emulation for presentation only (not the core framebuffer).
/// Reproduces AGB-001 washout via gamma plus primaries matrix; frontends opt in.
/// Framebuffer stays bit-exact BGR555 expansion.
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
    fn blend_cache_matches_direct_forms() {
        // Exhaustive differential: cached tables must equal the direct
        // formulas for every factor pair and channel combination.
        let mut cache = BlendCache::new();
        for eva in 0..=16u8 {
            for evb in 0..=16u8 {
                for a in 0..32u16 {
                    for b in 0..32u16 {
                        let table = blend_channel(a, b, eva, evb);
                        // Direct formula, mirrored from `alpha_blend`.
                        let direct = (((a as u32 * eva as u32 + b as u32 * evb as u32 + 8) >> 4)
                            .min(31)) as u16;
                        assert_eq!(table, direct);
                    }
                }
                // Spot-check assembled pixels through the cache entry.
                let first = 0x1234u16;
                let second = 0x5678u16;
                assert_eq!(
                    cache.alpha_blend(first, second, eva, evb),
                    alpha_blend(first, second, eva, evb)
                );
            }
        }
        for amount in 0..=16u8 {
            for v in 0..32u16 {
                let color = v | (v << 5) | (v << 10);
                assert_eq!(cache.brighten(color, amount), brighten(color, amount));
                assert_eq!(cache.darken(color, amount), darken(color, amount));
            }
        }
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
