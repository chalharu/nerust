use nerust_render_traits::logical::LogicalSize;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct FrameUploadLayout {
    pub(crate) bytes_per_pixel: u32,
    pub(crate) copy_bytes_per_row: u32,
    pub(crate) upload_bytes_per_row: u32,
    pub(crate) buffer_size: u64,
}

impl FrameUploadLayout {
    pub(crate) fn for_logical_size(
        logical_size: LogicalSize,
        bytes_per_pixel: u32,
    ) -> Result<Self, String> {
        let copy_bytes_per_row = logical_size
            .width
            .checked_mul(bytes_per_pixel as usize)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "frame upload row size overflowed u32".to_string())?;
        if copy_bytes_per_row == 0 || logical_size.height == 0 {
            return Err("empty frame upload".into());
        }
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let upload_bytes_per_row = copy_bytes_per_row
            .checked_add(alignment - 1)
            .map(|n| n / alignment * alignment)
            .ok_or_else(|| "frame upload row alignment overflowed u32".to_string())?;
        let buffer_size = u64::from(upload_bytes_per_row)
            .checked_mul(logical_size.height as u64)
            .ok_or_else(|| "frame upload buffer size overflowed u64".to_string())?;
        Ok(Self {
            bytes_per_pixel,
            copy_bytes_per_row,
            upload_bytes_per_row,
            buffer_size,
        })
    }
}

pub(crate) fn pack_frame_rows(
    source: &[u8],
    source_bytes_per_row: usize,
    height: usize,
    destination: &mut [u8],
    layout: FrameUploadLayout,
) -> Result<(), nerust_render_traits::FrameError> {
    let copy_bytes_per_row = layout.copy_bytes_per_row as usize;
    let upload_bytes_per_row = layout.upload_bytes_per_row as usize;
    nerust_render_traits::FrameLayout::checked(
        LogicalSize {
            width: copy_bytes_per_row / layout.bytes_per_pixel as usize,
            height,
        },
        layout.bytes_per_pixel as usize,
        source_bytes_per_row,
        source.len(),
    )?;
    let needed = upload_bytes_per_row
        .checked_mul(height)
        .ok_or(nerust_render_traits::FrameError::InvalidDimensions)?;
    if destination.len() < needed {
        return Err(nerust_render_traits::FrameError::TooShort {
            need: needed,
            have: destination.len(),
        });
    }
    destination[..needed].fill(0);
    for row in 0..height {
        destination[row * upload_bytes_per_row..row * upload_bytes_per_row + copy_bytes_per_row]
            .copy_from_slice(
                &source
                    [row * source_bytes_per_row..row * source_bytes_per_row + copy_bytes_per_row],
            );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nerust_render_traits::logical::LogicalSize;

    use super::{FrameUploadLayout, pack_frame_rows};

    #[test]
    fn final_row_padding_is_optional_extra_bytes_are_ignored_and_short_rows_rejected() {
        let layout = FrameUploadLayout {
            bytes_per_pixel: 4,
            copy_bytes_per_row: 8,
            upload_bytes_per_row: 16,
            buffer_size: 32,
        };
        for source in [vec![7; 20], vec![7; 99]] {
            let mut destination = [255; 32];
            pack_frame_rows(&source, 12, 2, &mut destination, layout).unwrap();
            assert_eq!(&destination[..8], &[7; 8]);
            assert_eq!(&destination[16..24], &[7; 8]);
            assert_eq!(&destination[8..16], &[0; 8]);
            assert_eq!(&destination[24..32], &[0; 8]);
        }
        for stride in [0, 4, 9, usize::MAX] {
            assert!(pack_frame_rows(&[7; 20], stride, 2, &mut [0; 32], layout).is_err());
        }
        assert!(pack_frame_rows(&[7; 19], 12, 2, &mut [0; 32], layout).is_err());
        assert!(pack_frame_rows(&[7; 20], 12, 2, &mut [0; 31], layout).is_err());
    }

    #[test]
    fn alignment_overflow_and_empty_layouts_return_errors() {
        for (width, height, bpp) in [
            (u32::MAX as usize, 1, 1),
            (0, 1, 4),
            (1, 0, 4),
            (usize::MAX, 1, 4),
        ] {
            assert!(
                FrameUploadLayout::for_logical_size(LogicalSize { width, height }, bpp).is_err()
            );
        }
    }

    #[test]
    fn aligned_upload_layout_keeps_native_row_pitch() {
        let layout = FrameUploadLayout::for_logical_size(
            LogicalSize {
                width: 256,
                height: 240,
            },
            4,
        )
        .expect("layout should be valid");

        assert_eq!(layout.bytes_per_pixel, 4);
        assert_eq!(layout.copy_bytes_per_row, 1024);
        assert_eq!(layout.upload_bytes_per_row, 1024);
        assert_eq!(layout.buffer_size, 245_760);
    }

    #[test]
    fn unaligned_upload_layout_rounds_up_to_copy_alignment() {
        let layout = FrameUploadLayout::for_logical_size(
            LogicalSize {
                width: 602,
                height: 240,
            },
            4,
        )
        .expect("layout should be valid");

        assert_eq!(layout.copy_bytes_per_row, 2408);
        assert_eq!(
            layout.upload_bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            0
        );
        assert_eq!(layout.upload_bytes_per_row, 2560);
        assert_eq!(layout.buffer_size, 614_400);
    }

    #[test]
    fn pack_frame_rows_inserts_row_padding_without_reordering_pixels() {
        let layout = FrameUploadLayout {
            bytes_per_pixel: 4,
            copy_bytes_per_row: 8,
            upload_bytes_per_row: 16,
            buffer_size: 32,
        };
        let source = [
            1_u8, 2, 3, 4, 5, 6, 7, 8, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let mut destination = [0_u8; 32];

        pack_frame_rows(&source, 8, 2, &mut destination, layout).unwrap();

        assert_eq!(&destination[0..8], &source[0..8]);
        assert_eq!(&destination[16..24], &source[8..16]);
        assert_eq!(&destination[8..16], &[0; 8]);
        assert_eq!(&destination[24..32], &[0; 8]);
    }

    #[test]
    fn pack_frame_rows_removes_source_padding() {
        let layout = FrameUploadLayout {
            bytes_per_pixel: 4,
            copy_bytes_per_row: 8,
            upload_bytes_per_row: 16,
            buffer_size: 32,
        };
        let source = [
            1_u8, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, //
            9, 10, 11, 12, 13, 14, 15, 16, 88, 88, 88, 88,
        ];
        let mut destination = [0_u8; 32];

        pack_frame_rows(&source, 12, 2, &mut destination, layout).unwrap();

        assert_eq!(&destination[0..8], &source[0..8]);
        assert_eq!(&destination[16..24], &source[12..20]);
        assert_eq!(&destination[8..16], &[0; 8]);
        assert_eq!(&destination[24..32], &[0; 8]);
    }
}
