#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone, Eq, PartialEq)]
pub struct RGB {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<u32> for RGB {
    fn from(value: u32) -> RGB {
        RGB {
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: (value & 0xFF) as u8,
        }
    }
}

/// Single RGBA word: canonical `0xRRGGBBAA` value on every host. All byte serialization and word
/// blends go through this type so no backend repeats endian-sensitive shifts or `*_bytes` conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbaWord(u32);

impl RgbaWord {
    /// Canonical constructor from channels; value is `0xRRGGBBAA`.
    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32))
    }

    /// Big-endian wire order `[R, G, B, A]` (palette/kernel blobs, GL uploads).
    pub const fn from_bytes_be(bytes: [u8; 4]) -> Self {
        Self(u32::from_be_bytes(bytes))
    }

    /// Big-endian wire order `[R, G, B, A]`; host-independent round-trip.
    pub const fn to_bytes_be(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// Native word whose *memory* is `[R, G, B, A]` (softbuffer resize taps).
    pub const fn as_ne(self) -> u32 {
        u32::from_ne_bytes(self.to_bytes_be())
    }

    /// Inverse of [`Self::as_ne`].
    pub const fn from_ne_word(word: u32) -> Self {
        Self::from_bytes_be(word.to_ne_bytes())
    }

    /// Canonical `0xRRGGBBAA` value for kernel sums and palette shifts.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Buffer word in the softbuffer `0xAARRGGBB` numeric convention.
    /// Single conversion site between the two word models.
    pub const fn as_argb_word(self) -> u32 {
        let [r, g, b, a] = self.to_bytes_be();
        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }

    /// Inverse of [`Self::as_argb_word`].
    pub const fn from_argb_word(word: u32) -> Self {
        Self::from_rgba(
            ((word >> 16) & 0xff) as u8,
            ((word >> 8) & 0xff) as u8,
            (word & 0xff) as u8,
            ((word >> 24) & 0xff) as u8,
        )
    }

    /// Canonical `0xRRGGBBAA` value (palette words, NTSC words, kernel sums).
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    /// Pack one tapped texel (`[R, G, B, A]` channels) into a native buffer word.
    pub const fn pack_argb(r: u8, g: u8, b: u8, a: u8) -> u32 {
        Self::from_rgba(r, g, b, a).as_argb_word()
    }

    /// Split one buffer word back into `[R, G, B, A]` channels.
    pub const fn unpack_argb(word: u32) -> [u8; 4] {
        Self::from_argb_word(word).to_bytes_be()
    }

    pub const fn r(self) -> u8 {
        (self.0 >> 24) as u8
    }

    pub const fn g(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn b(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn a(self) -> u8 {
        self.0 as u8
    }

    /// Straight-alpha composite of `src` over `self` in one word pass. Strength-reduced
    /// divide-by-255 (`(v*257 + 32895) >> 16`, exact for every `v` in range); alpha complement hoisted out of the channels.
    pub fn blend_src_over(self, src: [u8; 4]) -> Self {
        let alpha = u32::from(src[3]);
        // Fully opaque source replaces; fully transparent keeps dst.
        if alpha == 255 {
            return Self::from_rgba(src[0], src[1], src[2], 255);
        }
        if alpha == 0 {
            return self;
        }
        let inv = 255 - alpha;
        let mix = |s: u8, d: u8| {
            (((u32::from(s) * alpha + u32::from(d) * inv) * 257 + 32_895) >> 16) as u8
        };
        let [dr, dg, db, _] = self.to_bytes_be();
        Self::from_rgba(mix(src[0], dr), mix(src[1], dg), mix(src[2], db), 255)
    }
}

#[cfg(test)]
mod tests {
    use super::RgbaWord;

    #[test]
    fn be_bytes_round_trip_on_any_host() {
        // Big-endian wire order is host-independent by construction.
        let word = RgbaWord::from_rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(word.to_bytes_be(), [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(RgbaWord::from_bytes_be([0x12, 0x34, 0x56, 0x78]), word);
        assert_eq!(word.0, 0x1234_5678);
    }

    #[test]
    fn ne_word_preserves_rgba_memory_order() {
        let word = RgbaWord::from_rgba(0x11, 0x22, 0x33, 0x44);
        assert_eq!(RgbaWord::from_ne_word(word.as_ne()), word);
        assert_eq!(word.as_ne().to_ne_bytes(), [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn argb_word_matches_softbuffer_convention() {
        // Opaque red renders as `0xFFFF_0000` in the legacy buffer model.
        assert_eq!(
            RgbaWord::from_rgba(255, 0, 0, 255).as_argb_word(),
            0xFFFF_0000
        );
        assert_eq!(
            RgbaWord::from_argb_word(0xFFFF_0000),
            RgbaWord::from_rgba(255, 0, 0, 255)
        );
    }

    #[test]
    fn blend_matches_scalar_integer_math() {
        // Exhaustive over every reachable numerator: the strength-reduced
        // `(v*257 + 32895) >> 16` form equals `(v + 127) / 255` bit-for-bit.
        for v in 0..=65_025u32 {
            assert_eq!(((v * 257 + 32_895) >> 16) as u8, ((v + 127) / 255) as u8);
        }
        // Exhaustive over alpha: word blend equals the scalar formula.
        let dst = RgbaWord::from_rgba(10, 20, 30, 255);
        for alpha in 0..=255u8 {
            let src = [200u8, 100, 50, alpha];
            let got = dst.blend_src_over(src);
            let mix = |s: u8, d: u8| {
                ((u32::from(s) * u32::from(alpha) + u32::from(d) * (255 - u32::from(alpha)) + 127)
                    / 255) as u8
            };
            let [er, eg, eb, _] = if alpha == 255 {
                [src[0], src[1], src[2], 255]
            } else if alpha == 0 {
                [10, 20, 30, 255]
            } else {
                [mix(src[0], 10), mix(src[1], 20), mix(src[2], 30), 255]
            };
            assert_eq!(got.to_bytes_be(), [er, eg, eb, 255], "alpha={alpha}");
        }
    }

    #[test]
    fn pack_helpers_pin_native_argb_layout() {
        // Opaque red over opaque black stays red; alpha byte stays set.
        assert_eq!(RgbaWord::pack_argb(255, 0, 0, 255), 0xFFFF_0000);
        assert_eq!(RgbaWord::unpack_argb(0xFFFF_0000), [255, 0, 0, 255]);
        // Half red over black: (255*128 + 0*127 + 127) / 255 = 128.
        assert_eq!(RgbaWord::pack_argb(128, 0, 0, 255), 0xFF80_0000);
        // `RgbaWord` round-trips on every host by construction.
        assert_eq!(RgbaWord::pack_argb(0x12, 0x34, 0x56, 0x78), 0x7812_3456);
        assert_eq!(RgbaWord::unpack_argb(0x7812_3456), [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn argb_and_channel_accessors_round_trip() {
        let word = RgbaWord::from_rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(word.as_u32(), 0x1234_5678);
        assert_eq!(word.r(), 0x12);
        assert_eq!(word.g(), 0x34);
        assert_eq!(word.b(), 0x56);
        assert_eq!(word.a(), 0x78);
        assert_eq!(RgbaWord::from_argb_word(word.as_argb_word()), word);
    }

    #[test]
    fn ne_word_preserves_host_memory() {
        let word = RgbaWord::from_rgba(1, 2, 3, 4);
        assert_eq!(word.as_ne().to_ne_bytes(), [1, 2, 3, 4]);
        assert_eq!(RgbaWord::from_ne_word(word.as_ne()), word);
    }
}
