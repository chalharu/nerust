use png::{BitDepth, ColorType, Encoder};

use crate::error::PersistenceError;

const THUMBNAIL_TARGET_WIDTH: u32 = 320;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailSource {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn encode_thumbnail_png(source: &ThumbnailSource) -> Result<Vec<u8>, PersistenceError> {
    if source.width == 0 || source.height == 0 {
        return Err(PersistenceError::Validation(
            "thumbnail source dimensions must be non-zero".into(),
        ));
    }
    if source.rgba.len() != (source.width as usize) * (source.height as usize) * 4 {
        return Err(PersistenceError::Validation(
            "thumbnail RGBA buffer length mismatch".into(),
        ));
    }

    let target_width = THUMBNAIL_TARGET_WIDTH;
    let target_height =
        ((u64::from(source.height) * u64::from(target_width)) / u64::from(source.width)) as u32;
    let target_height = target_height.max(1);
    let resized = resize_rgba_nearest(source, target_width, target_height);

    let mut png_bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_bytes, target_width, target_height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&resized)?;
    }

    Ok(png_bytes)
}

fn resize_rgba_nearest(source: &ThumbnailSource, width: u32, height: u32) -> Vec<u8> {
    let mut resized = vec![0; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        let src_y = (u64::from(y) * u64::from(source.height) / u64::from(height)) as usize;
        for x in 0..width {
            let src_x = (u64::from(x) * u64::from(source.width) / u64::from(width)) as usize;
            let src_offset = (src_y * source.width as usize + src_x) * 4;
            let dst_offset = (y as usize * width as usize + x as usize) * 4;
            resized[dst_offset..dst_offset + 4]
                .copy_from_slice(&source.rgba[src_offset..src_offset + 4]);
        }
    }
    resized
}
