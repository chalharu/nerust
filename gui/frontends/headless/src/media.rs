use std::{
    hash::{Hash, Hasher},
    io::Cursor,
};

use crc::{CRC_64_XZ, Crc, Digest};
use nerust_core_traits::audio::{AudioBackend, StereoSample};
use nerust_render_traits::{FrameBuffer, PixelFormat};
use png::{BitDepth, ColorType, Encoder};

use super::error::RomTestError;

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

/// Validation frame buffer carrying the factory palette.
///
/// The palette is display metadata only: hashes cover indices, and PNG
/// encoding derives RGBA from these same bytes. It must come from the
/// factory (filter pinned to `None` at construction), never rebuilt here.
pub(crate) fn validation_screen_buffer(palette: &[u32]) -> FrameBuffer {
    let mut owned = [0u32; 256];
    let n = palette.len().min(256);
    owned[..n].copy_from_slice(&palette[..n]);
    let mut fb = FrameBuffer::with_capacity(
        256,
        240,
        PixelFormat::PaletteIndex {
            palette: Box::new(owned),
        },
    );
    fb.resize(256, 240);
    fb
}

pub(crate) fn screen_hash(frame: &FrameBuffer) -> u64 {
    let mut hasher = Crc64Hasher::new();
    frame.as_ref().hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn encode_screenshot_png(frame: &FrameBuffer) -> Result<Vec<u8>, RomTestError> {
    let w = frame.width();
    let h = frame.height();
    let src = frame.as_ref();
    let palette_rgba8 = match frame.palette_as_rgba8() {
        Some(palette) => palette,
        None => {
            return Err(RomTestError::InvalidManifest(
                "screenshot buffer carries no palette".to_string(),
            ));
        }
    };
    let mut rgba = Vec::with_capacity(w * h * 4);

    for &index in src.iter().take(w * h) {
        let i = usize::from(index.min(63)) * 4;
        rgba.push(palette_rgba8[i]);
        rgba.push(palette_rgba8[i + 1]);
        rgba.push(palette_rgba8[i + 2]);
        rgba.push(palette_rgba8[i + 3]);
    }

    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = Encoder::new(&mut encoded, w as u32, h as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
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

impl AudioBackend for HashingMixer {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, data: StereoSample) {
        assert_eq!(data.left.to_bits(), data.right.to_bits());
        self.samples += 1;
        self.checksum ^= u64::from(data.left.to_bits());
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
