//! Pixel word vocabulary: one pack/unpack pair for the whole backend.
//! Buffer words are native-endian `0xAARRGGBB`; the resize taps and the OSD
//! blend compose channels in `[B, G, R, A]` order. Helpers use `ne_bytes`
//! so the layout is explicit in one place instead of scattered conversions.

/// Pack one tapped texel (`[B, G, R, A]` bytes) into a native buffer word.
pub(crate) fn pack_word(b: u8, g: u8, r: u8, a: u8) -> u32 {
    u32::from_ne_bytes([b, g, r, a])
}

/// Split one buffer word back into `[B, G, R, A]` tap bytes.
pub(crate) fn unpack_word(word: u32) -> [u8; 4] {
    word.to_ne_bytes()
}

#[cfg(test)]
mod tests {
    use super::{pack_word, unpack_word};

    #[test]
    fn words_pin_native_argb_layout() {
        // Opaque red over opaque black stays red; alpha byte stays set.
        assert_eq!(pack_word(0, 0, 255, 255), 0xFFFF_0000);
        assert_eq!(unpack_word(0xFFFF_0000), [0, 0, 255, 255]);
        // Half red over black: (255*128 + 0*127 + 127) / 255 = 128.
        assert_eq!(pack_word(0, 0, 128, 255), 0xFF80_0000);
        // `ne_bytes` round-trips on this platform by construction.
        assert_eq!(pack_word(0x12, 0x34, 0x56, 0x78), 0x7856_3412);
        assert_eq!(unpack_word(0x7856_3412), [0x12, 0x34, 0x56, 0x78]);
    }
}
