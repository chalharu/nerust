//! Frame texture domain: allocation-free initial uploads plus the
//! NTSC kernel path. [`super::view::GlView`] coordinates; this module
//! owns the TexImage calls so the view never host-memsets a null
//! upload.

use gl::types::{GLenum, GLint};
use nerust_glwrap::raw::{active_texture, bind_texture, tex_image_2d, tex_parameteri};

use crate::error::ViewError;

fn ok_gl<T>(result: Result<T, nerust_glwrap::GlError>) -> Result<T, ViewError> {
    result.map_err(ViewError::Gl)
}

/// Point-sampled clamp addressing, shared by all upload textures.
pub(crate) fn set_nearest_params() -> Result<(), ViewError> {
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_WRAP_S,
        gl::CLAMP_TO_EDGE as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_WRAP_T,
        gl::CLAMP_TO_EDGE as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_MAG_FILTER,
        gl::NEAREST as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_MIN_FILTER,
        gl::NEAREST as i32,
    ))?;
    Ok(())
}

/// Bind one texture unit; the active/bind pair repeats at uploads.
pub(crate) fn use_texture(unit: u32, texture: u32) -> Result<(), ViewError> {
    ok_gl(active_texture(gl::TEXTURE0 + unit))?;
    ok_gl(bind_texture(gl::TEXTURE_2D, texture))?;
    Ok(())
}

fn checked_i32(value: usize) -> Result<i32, ViewError> {
    i32::try_from(value)
        .map_err(|_| ViewError::Frame(crate::layout::FrameError::StrideTooWide { have: value }))
}

/// Allocate the texture store with null data: no host memset, no
/// zero-fill staging. Real rows arrive via `TexSubImage` updates.
pub(crate) fn allocate_texture_store(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    internal_fmt: GLint,
    data_fmt: GLenum,
) -> Result<(), ViewError> {
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(tex_image_2d(
        gl::TEXTURE_2D,
        0,
        internal_fmt,
        checked_i32(width)?,
        checked_i32(height)?,
        0,
        data_fmt,
        gl::UNSIGNED_BYTE,
        std::ptr::null(),
    ))?;
    Ok(())
}

/// NTSC kernel texture (RGBA8) upload path.
///
/// # Canonical byte order
///
/// The packed kernel (`be_data`) is a sequence of big-endian `u32`
/// words: bytes `[R, G, B, A]` in that order on the wire. The upload
/// below copies those bytes verbatim into the RGBA8 texture, so the
/// GPU samples exactly what the CPU packed: word
/// `u32::from_be_bytes([R, G, B, A])`. This matches the softbuffer
/// CPU path, which indexes the same packing with `to_be_bytes` (see
/// `read_entry` there). A cross-check test
/// (`ntsc_upload_bytes_match_cpu_order`) locks the identity: upload
/// bytes must equal the CPU indexer order byte-for-byte.
/// Pure pack step of the NTSC kernel upload: verbatim big-endian
/// byte copy with zero padding, no GL. Returns `(actual_height,
/// need)`; the caller uploads `scratch[..need]`.
/// Unit-tested without a context (see `ntsc_pack_*` below).
pub(crate) fn pack_ntsc_bytes(
    width: usize,
    height: usize,
    be_data: &[u8],
    scratch: &mut Vec<u8>,
) -> Result<(usize, usize), ViewError> {
    // Row stride, then total bytes: each step checked so a corrupt
    // kernel header refuses instead of wrapping the scratch.
    let row_bytes = width.checked_mul(4).ok_or(ViewError::from(
        crate::layout::FrameError::DimensionsOverflow {
            have: be_data.len(),
        },
    ))?;
    let entry_count = be_data.len() / row_bytes.max(1);
    let actual_height = entry_count.max(height);
    let need = actual_height.checked_mul(row_bytes).ok_or(ViewError::from(
        crate::layout::FrameError::DimensionsOverflow {
            have: be_data.len(),
        },
    ))?;
    if scratch.len() < need {
        scratch.resize(need, 0);
    }
    let rgba = &mut scratch[..need];
    for row in 0..actual_height {
        for col in 0..width {
            let offset = (row * width + col) * 4;
            if offset + 4 <= be_data.len() {
                rgba[offset..offset + 4].copy_from_slice(&be_data[offset..offset + 4]);
            } else {
                rgba[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Ok((actual_height, need))
}

pub(crate) fn configure_ntsc_texture(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    be_data: &[u8],
    scratch: &mut Vec<u8>,
) -> Result<(), ViewError> {
    let (actual_height, need) = pack_ntsc_bytes(width, height, be_data, scratch)?;
    upload_texture_bytes(
        unit,
        texture,
        width,
        actual_height,
        gl::RGBA as GLint,
        gl::RGBA,
        &scratch[..need],
    )?;
    Ok(())
}

/// Initial upload with caller bytes (placeholder palettes, kernels).
pub(crate) fn upload_texture_bytes(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    internal_fmt: GLint,
    data_fmt: GLenum,
    data: &[u8],
) -> Result<(), ViewError> {
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(tex_image_2d(
        gl::TEXTURE_2D,
        0,
        internal_fmt,
        checked_i32(width)?,
        checked_i32(height)?,
        0,
        data_fmt,
        gl::UNSIGNED_BYTE,
        data.as_ptr().cast(),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pack_ntsc_bytes;

    #[test]
    fn ntsc_pack_copies_bytes_verbatim_with_zero_pad() {
        // Three full words: verbatim copy, tail zero-padded. A short
        // trailing partial word never copies (offset guard).
        let be: Vec<u8> = (1u8..=12).collect();
        let mut scratch = Vec::new();
        let (height, need) = pack_ntsc_bytes(2, 2, &be, &mut scratch).unwrap();
        assert_eq!((height, need), (2, 16));
        assert_eq!(&scratch[..12], &(1u8..=12).collect::<Vec<_>>()[..]);
        assert_eq!(&scratch[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn ntsc_upload_bytes_match_cpu_order() {
        // Canonical order lock: the GPU upload bytes equal the CPU
        // indexer order byte-for-byte, i.e. word
        // `u32::from_be_bytes([R, G, B, A])` and back via
        // `to_be_bytes` (the softbuffer `read_entry` convention).
        let word = u32::from_be_bytes([0x11, 0x22, 0x33, 0x44]);
        assert_eq!(word.to_be_bytes(), [0x11, 0x22, 0x33, 0x44]);
        let be = word.to_be_bytes();
        let mut scratch = Vec::new();
        let (_, need) = pack_ntsc_bytes(1, 1, &be, &mut scratch).unwrap();
        assert_eq!(need, 4);
        assert_eq!(&scratch[..need], &be);
        assert_eq!(u32::from_be_bytes(scratch[..4].try_into().unwrap()), word);
    }

    #[test]
    fn ntsc_pack_rejects_overflow() {
        let mut scratch = Vec::new();
        let err = pack_ntsc_bytes(usize::MAX, usize::MAX, &[0; 4], &mut scratch).unwrap_err();
        assert!(err.to_string().contains("overflow"));
    }
}
