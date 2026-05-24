// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_screen_traits::logical_size::LogicalSize;

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
        let upload_bytes_per_row =
            align_copy_bytes_per_row(copy_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
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

fn align_copy_bytes_per_row(bytes_per_row: u32, alignment: u32) -> u32 {
    bytes_per_row.div_ceil(alignment) * alignment
}

pub(crate) fn pack_frame_rows(
    source: &[u8],
    height: usize,
    destination: &mut [u8],
    layout: FrameUploadLayout,
) {
    let copy_bytes_per_row = layout.copy_bytes_per_row as usize;
    let upload_bytes_per_row = layout.upload_bytes_per_row as usize;
    debug_assert_eq!(source.len(), copy_bytes_per_row * height);
    debug_assert!(destination.len() >= upload_bytes_per_row * height);

    if copy_bytes_per_row == upload_bytes_per_row {
        destination[..source.len()].copy_from_slice(source);
        return;
    }

    for (source_row, destination_row) in source
        .chunks_exact(copy_bytes_per_row)
        .zip(destination.chunks_exact_mut(upload_bytes_per_row))
        .take(height)
    {
        destination_row[..copy_bytes_per_row].copy_from_slice(source_row);
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameUploadLayout, pack_frame_rows};
    use nerust_screen_traits::logical_size::LogicalSize;

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

        pack_frame_rows(&source, 2, &mut destination, layout);

        assert_eq!(&destination[0..8], &source[0..8]);
        assert_eq!(&destination[16..24], &source[8..16]);
        assert_eq!(&destination[8..16], &[0; 8]);
        assert_eq!(&destination[24..32], &[0; 8]);
    }
}
