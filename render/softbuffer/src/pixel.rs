//! Pixel word vocabulary: one pack/unpack pair for the whole backend. Buffer words are numeric
//! `0xAARRGGBB` (memory `[B, G, R, A]`); both directions go through [`RgbaWord`] so the layout lives in one place.

use nerust_render_traits::rgb::RgbaWord;

/// Pack one tapped texel (`[R, G, B, A]` channels) into a native buffer word.
pub(crate) fn pack_word(r: u8, g: u8, b: u8, a: u8) -> u32 {
    RgbaWord::from_rgba(r, g, b, a).as_argb_word()
}

/// Split one buffer word back into `[R, G, B, A]` channels.
pub(crate) fn unpack_word(word: u32) -> [u8; 4] {
    RgbaWord::from_argb_word(word).to_bytes_be()
}

#[cfg(test)]
mod tests {
    use super::{pack_word, unpack_word};

    #[test]
    fn words_pin_native_argb_layout() {
        // Opaque red over opaque black stays red; alpha byte stays set.
        assert_eq!(pack_word(255, 0, 0, 255), 0xFFFF_0000);
        assert_eq!(unpack_word(0xFFFF_0000), [255, 0, 0, 255]);
        // Half red over black: (255*128 + 0*127 + 127) / 255 = 128.
        assert_eq!(pack_word(128, 0, 0, 255), 0xFF80_0000);
        // `RgbaWord` round-trips on every host by construction.
        assert_eq!(pack_word(0x12, 0x34, 0x56, 0x78), 0x7812_3456);
        assert_eq!(unpack_word(0x7812_3456), [0x12, 0x34, 0x56, 0x78]);
    }
}
