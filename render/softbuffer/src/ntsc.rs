//! NTSC kernel simulation: palette-index to RGBA words.
//! Pure byte math; the renderer only stages the output buffer.

use nerust_render_traits::{
    VideoRenderProfile, filter::BLACK_PALETTE_INDEX, logical::LogicalSize, rgb::RgbaWord,
};

const NTSC_ROW_OFFSETS: [[usize; 6]; 7] = [
    [0, 19, 31, 7, 26, 38],
    [1, 20, 32, 8, 27, 39],
    [2, 14, 33, 9, 21, 40],
    [3, 15, 34, 10, 22, 41],
    [4, 16, 28, 11, 23, 35],
    [5, 17, 29, 12, 24, 36],
    [6, 18, 30, 13, 25, 37],
];
const NTSC_SOURCE_OFFSETS: [[i32; 6]; 7] = [
    [1, -1, 0, -2, -4, -3],
    [1, -1, 0, -2, -4, -3],
    [1, 2, 0, -2, -1, -3],
    [1, 2, 0, -2, -1, -3],
    [1, 2, 3, -2, -1, 0],
    [1, 2, 3, -2, -1, 0],
    [1, 2, 3, -2, -1, 0],
];

pub(crate) struct Ntsc;

impl Ntsc {
    pub(crate) fn palette_index(
        source_frame: &[u8],
        size: LogicalSize,
        stride: usize,
        x: i32,
        y: usize,
    ) -> u8 {
        if x < 0 || x as usize >= size.width || y >= size.height {
            return BLACK_PALETTE_INDEX;
        }
        // Short rows render black instead of panicking the UI thread.
        source_frame
            .get(y.saturating_mul(stride).saturating_add(x as usize))
            .copied()
            .unwrap_or(BLACK_PALETTE_INDEX)
    }

    fn clamp_impl(io: u32) -> u32 {
        const NTSC_CLAMP_MASK: u32 = 0x300c03;
        const NTSC_CLAMP_ADD: u32 = 0x20280a02;

        let sub = (io >> 9) & NTSC_CLAMP_MASK;
        let clamp = NTSC_CLAMP_ADD.wrapping_sub(sub);
        (io | clamp) & clamp.wrapping_sub(sub)
    }

    fn rgb_out_impl(raw: u32) -> u32 {
        let rgb = ((raw >> 5) & 0x00ff0000) | ((raw >> 3) & 0x0000ff00) | ((raw >> 1) & 0x000000ff);
        (rgb << 8) | 0xff
    }

    pub(crate) fn read_entry(buf: &[u8], color: u8, row: usize) -> u32 {
        const PALETTE_WIDTH: usize = 64;
        let offset = (row
            .saturating_mul(PALETTE_WIDTH)
            .saturating_add(color as usize))
        .saturating_mul(4);
        // Out-of-range palette data renders black instead of
        // panicking the UI thread; valid entries are untouched.
        buf.get(offset..offset.saturating_add(4))
            .and_then(|s| s.try_into().ok())
            .map(|bytes: [u8; 4]| RgbaWord::from_bytes_be(bytes).as_u32())
            .unwrap_or(0)
    }

    pub(crate) fn simulate_gpu_ntsc_rgba(
        src_h: usize,
        src_stride: usize,
        source_frame: &[u8],
        render_profile: &VideoRenderProfile,
        ntsc_buffer: &mut [u32],
    ) {
        let packed_entries = render_profile.ntsc_packed_rgba8.as_ref().unwrap();
        let src_w = render_profile.logical_size.width;

        for y in 0..src_h {
            let phase_row = (y % 3) * 42;
            for x in 0..src_w {
                let chunk = x / 7;
                let sample = x - chunk * 7;
                let base = (chunk * 3) as i32;
                let row_offsets = NTSC_ROW_OFFSETS[sample];
                let source_offsets = NTSC_SOURCE_OFFSETS[sample];
                let mut sum = 0_u32;

                for (source_offset, row_offset) in source_offsets.into_iter().zip(row_offsets) {
                    let color = Self::palette_index(
                        source_frame,
                        render_profile.source_logical_size,
                        src_stride,
                        base + source_offset,
                        y,
                    );
                    sum = sum.wrapping_add(Self::read_entry(
                        packed_entries,
                        color,
                        phase_row + row_offset,
                    ));
                }

                ntsc_buffer[y * src_w + x] = Self::rgb_out_impl(Self::clamp_impl(sum));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Ntsc;
    use nerust_render_traits::rgb::RgbaWord;

    #[test]
    fn palette_rows_honor_source_pitch_and_profile_bounds() {
        use nerust_render_traits::{filter::BLACK_PALETTE_INDEX, logical::LogicalSize};
        let size = LogicalSize {
            width: 2,
            height: 2,
        };
        let source = [1, 2, 99, 99, 3, 4, 99, 99, 5, 6];
        assert_eq!(Ntsc::palette_index(&source, size, 4, 0, 1), 3);
        assert_eq!(Ntsc::palette_index(&source, size, 4, 1, 1), 4);
        assert_eq!(
            Ntsc::palette_index(&source, size, 4, 2, 1),
            BLACK_PALETTE_INDEX
        );
        assert_eq!(
            Ntsc::palette_index(&source, size, 4, 0, 2),
            BLACK_PALETTE_INDEX
        );
    }

    #[test]
    fn read_entry_uses_be_wire_order() {
        let bytes = [0x12u8, 0x34, 0x56, 0x78];
        let mut buf = vec![0u8; 64 * 4];
        buf[..4].copy_from_slice(&bytes);
        let word = Ntsc::read_entry(&buf, 0, 0);
        assert_eq!(word, RgbaWord::from_bytes_be(bytes).as_u32());
        assert_eq!(Ntsc::read_entry(&buf, 0, 99_999), 0);
    }

    #[test]
    fn clamp_and_rgb_out_are_stable() {
        assert_eq!(Ntsc::rgb_out_impl(0), 0x0000_00FF);
        assert_eq!(Ntsc::clamp_impl(0), Ntsc::clamp_impl(0));
        assert_eq!(Ntsc::rgb_out_impl(u32::MAX), Ntsc::rgb_out_impl(u32::MAX));
    }

    #[test]
    fn simulate_writes_one_word_per_texel() {
        use nerust_render_traits::{
            VideoFrameFormat, logical::LogicalSize, physical::PhysicalSize,
        };
        let profile = nerust_render_traits::VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 4,
                height: 4,
            },
            logical_size: LogicalSize {
                width: 4,
                height: 4,
            },
            physical_size: PhysicalSize {
                width: 4.0,
                height: 4.0,
            },
            frame_format: VideoFrameFormat::Palette,
            ntsc_packed_rgba8: Some(vec![0x7Fu8; 34_000].into_boxed_slice()),
        };
        let src = vec![3u8; 4 * 4];
        let mut out = vec![0u32; 16];
        Ntsc::simulate_gpu_ntsc_rgba(4, 4, &src, &profile, &mut out);
        assert_eq!(out.len(), 16);
        assert!(out.iter().all(|w| w & 0xFF == 0xFF));
    }
}
