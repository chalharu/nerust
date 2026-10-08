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

pub(crate) fn screen_hash(frame: &FrameBuffer) -> u64 {
    let mut hasher = Crc64Hasher::new();
    frame.as_ref().hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn encode_screenshot_png(frame: &FrameBuffer) -> Result<Vec<u8>, RomTestError> {
    let w = frame.width();
    let h = frame.height();
    let src = frame.as_ref();
    // Hashing stays format-agnostic (raw bytes); only the PNG needs
    // per-format decoding. Rgba systems (GBC/GBA) carry no palette.
    let mut rgba = Vec::with_capacity(w * h * 4);

    match frame.format() {
        PixelFormat::Rgba => {
            // stride is bytes per row (256-byte aligned for Rgba).
            let stride = frame.stride();
            for y in 0..h {
                let start = y * stride;
                // Short rows yield short output; the encoder rejects
                // the length mismatch loudly below — never a panic.
                rgba.extend_from_slice(src.get(start..start + w * 4).unwrap_or_default());
            }
        }
        PixelFormat::PaletteIndex { .. } => {
            let palette_rgba8 = match frame.palette_as_rgba8() {
                Some(palette) => palette,
                None => {
                    return Err(RomTestError::InvalidManifest(
                        "screenshot buffer carries no palette".to_string(),
                    ));
                }
            };
            for &index in src.iter().take(w * h) {
                let i = usize::from(index.min(63)) * 4;
                rgba.push(palette_rgba8[i]);
                rgba.push(palette_rgba8[i + 1]);
                rgba.push(palette_rgba8[i + 2]);
                rgba.push(palette_rgba8[i + 3]);
            }
        }
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
        // The mixer tracks the left channel only. NES-family streams
        // are dual-mono (left == right), so their hashes are unaffected;
        // stereo systems pin the left channel while the right stays
        // unobserved (documented gap, not a silent assumption: the old
        // assert crashed on the first stereo case instead).
        self.samples += 1;
        self.checksum ^= u64::from(data.left.to_bits());
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_rgba8(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(Cursor::new(png));
        let mut reader = decoder.read_info().expect("valid png");
        let mut buf = vec![0; reader.output_buffer_size().expect("sized")];
        let info = reader.next_frame(&mut buf).expect("frame");
        assert_eq!(info.color_type, ColorType::Rgba);
        assert_eq!(info.bit_depth, BitDepth::Eight);
        (info.width, info.height, buf)
    }

    fn rgba_frame(pixels: &[u8; 16]) -> FrameBuffer {
        let mut frame = FrameBuffer::with_capacity(2, 2, PixelFormat::Rgba);
        frame.resize(2, 2);
        // Rgba rows are stride-separated (bytes per row, aligned);
        // place each row at its strided offset.
        let stride = frame.stride();
        let buf = frame.as_mut();
        buf[..8].copy_from_slice(&pixels[..8]);
        buf[stride..stride + 8].copy_from_slice(&pixels[8..]);
        frame
    }

    #[test]
    fn encode_rgba_buffer_without_palette() {
        // GBC/GBA-class buffers: no palette, bytes pass through.
        let pixels = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 255, 255, // white
        ];
        let frame = rgba_frame(&pixels);
        let png = encode_screenshot_png(&frame).expect("rgba encodes");
        let (w, h, decoded) = decode_rgba8(&png);
        assert_eq!((w, h), (2, 2));
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn encode_palette_buffer_maps_indices() {
        let mut palette = vec![0u32; 256].into_boxed_slice();
        palette[0] = 0xFF00_00FF; // red, opaque
        palette[1] = 0x00FF_00FF; // green, opaque
        let mut frame = FrameBuffer::with_capacity(2, 1, PixelFormat::PaletteIndex { palette });
        frame.resize(2, 1);
        frame.resize_data(2);
        frame.as_mut().copy_from_slice(&[0, 1]);
        let png = encode_screenshot_png(&frame).expect("palette encodes");
        let (w, h, decoded) = decode_rgba8(&png);
        assert_eq!((w, h), (2, 1));
        assert_eq!(decoded, [255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn short_rgba_buffer_fails_loudly() {
        // Truncated source must error from the encoder, never panic.
        let mut frame = FrameBuffer::with_capacity(2, 2, PixelFormat::Rgba);
        frame.resize(2, 2);
        frame.resize_data(4); // one pixel of four
        let result = encode_screenshot_png(&frame);
        assert!(result.is_err(), "short buffer should not encode");
    }
}
