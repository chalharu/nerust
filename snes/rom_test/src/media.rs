// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crc::{CRC_64_XZ, Crc, Digest};
use png::{BitDepth, ColorType, Encoder};
use std::hash::Hasher;
use std::io::Cursor;

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 224;

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

pub fn screen_hash_rgba(rgba: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.write(rgba);
    hasher.finish()
}

pub fn encode_screenshot_png(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, png::EncodingError> {
    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    drop(writer);
    Ok(encoded.into_inner())
}

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

impl Hasher for Crc64Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        self.0.clone().finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::screen_hash_rgba;

    #[test]
    fn screen_hash_changes_with_pixel_content() {
        let first = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x60, 0xFF];
        let second = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x61, 0xFF];

        assert_eq!(screen_hash_rgba(&first), screen_hash_rgba(&first));
        assert_ne!(screen_hash_rgba(&first), screen_hash_rgba(&second));
    }
}
