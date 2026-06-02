// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::error::RomTestError;
use nerust_crc64_hasher::Crc64Hasher;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_logical::LogicalSize;
use nerust_sound_traits::MixerInput;
use png::{BitDepth, ColorType, Encoder};
use std::hash::{Hash, Hasher};
use std::io::Cursor;

pub(crate) fn validation_screen_buffer() -> ScreenBuffer {
    ScreenBuffer::new(
        FilterType::None,
        LogicalSize {
            width: 256,
            height: 240,
        },
    )
}

pub(crate) fn screen_hash(screen_buffer: &ScreenBuffer) -> u64 {
    let mut hasher = Crc64Hasher::new();
    screen_buffer.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn encode_screenshot_png(screen_buffer: &ScreenBuffer) -> Result<Vec<u8>, RomTestError> {
    let logical_size = screen_buffer.logical_size();
    let mut buffer = vec![0_u8; screen_buffer.frame_len()];
    screen_buffer.copy_display_buffer(&mut buffer);
    let mut rgba = Vec::with_capacity(buffer.len());

    for pixel in buffer.chunks_exact(4) {
        let value = u32::from_ne_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
        rgba.push((value & 0xFF) as u8);
        rgba.push(((value >> 8) & 0xFF) as u8);
        rgba.push(((value >> 16) & 0xFF) as u8);
        rgba.push(((value >> 24) & 0xFF) as u8);
    }

    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(
        &mut encoded,
        logical_size.width as u32,
        logical_size.height as u32,
    );
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    drop(writer);

    Ok(encoded.into_inner())
}

#[derive(Debug, Clone)]
pub(crate) struct HashingMixer {
    sample_rate: u32,
    samples: u64,
    checksum: u64,
}

impl HashingMixer {
    const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            samples: 0,
            checksum: Self::FNV_OFFSET_BASIS,
        }
    }

    pub(crate) fn samples(&self) -> u64 {
        self.samples
    }

    pub(crate) fn checksum(&self) -> u64 {
        self.checksum
    }
}

impl MixerInput for HashingMixer {
    fn push(&mut self, data: f32) {
        self.samples += 1;
        self.checksum ^= u64::from(data.to_bits());
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
