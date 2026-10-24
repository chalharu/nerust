//! Frame texture allocation and validated CPU uploads, including NTSC kernels.
//! This module owns the TexImage calls; the renderer coordinates the views.

use gl::types::{GLenum, GLint};
use nerust_glwrap::bindings as gl;
use nerust_glwrap::raw::{
    active_texture, bind_buffer, bind_texture, pixel_storei, tex_image_2d, tex_parameteri,
};
use nerust_render_traits::rgb::RgbaWord;

use crate::upload::StagingBuf;

use crate::error::ViewError;

fn ok_gl<T>(result: Result<T, nerust_glwrap::GlError>) -> Result<T, ViewError> {
    result.map_err(ViewError::Gl)
}

/// CPU slice uploads on the renderer's GL 3.3 context must not inherit PBO
/// offsets, skipped rows/pixels, or a foreign row pitch. Length validation is
/// meaningful only after establishing these unpack invariants.
pub(crate) fn prepare_cpu_unpack() -> Result<(), ViewError> {
    ok_gl(bind_buffer(gl::PIXEL_UNPACK_BUFFER, 0))?;
    ok_gl(pixel_storei(gl::UNPACK_ALIGNMENT, 1))?;
    ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, 0))?;
    ok_gl(pixel_storei(gl::UNPACK_SKIP_ROWS, 0))?;
    ok_gl(pixel_storei(gl::UNPACK_SKIP_PIXELS, 0))?;
    Ok(())
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
    prepare_cpu_unpack()?;
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(unsafe {
        tex_image_2d(
            gl::TEXTURE_2D,
            0,
            internal_fmt,
            checked_i32(width)?,
            checked_i32(height)?,
            0,
            data_fmt,
            gl::UNSIGNED_BYTE,
            std::ptr::null(),
        )
    })?;
    Ok(())
}

/// NTSC kernel texture (RGBA8) upload path: verbatim big-endian byte copy.
/// Pure pack step through [`RgbaWord`]; returns `(actual_height, need)`.
pub(crate) fn pack_ntsc_bytes(
    width: usize,
    height: usize,
    be_data: &[u8],
    scratch: &mut StagingBuf,
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
    let rgba = scratch.staging(need);
    for row in 0..actual_height {
        for col in 0..width {
            let offset = (row * width + col) * 4;
            let word = match be_data.get(offset..offset + 4) {
                Some(bytes) => RgbaWord::from_bytes_be(bytes.try_into().expect("4-byte word")),
                None => RgbaWord::from_rgba(0, 0, 0, 0),
            };
            rgba[offset..offset + 4].copy_from_slice(&word.to_bytes_be());
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
    scratch: &mut StagingBuf,
) -> Result<(), ViewError> {
    let (actual_height, need) = pack_ntsc_bytes(width, height, be_data, scratch)?;
    upload_texture_bytes(
        unit,
        texture,
        width,
        actual_height,
        gl::RGBA as GLint,
        gl::RGBA,
        &scratch.staging(need)[..need],
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
    let bpp = match data_fmt {
        gl::RGBA => 4,
        gl::RED => 1,
        _ => return Err(crate::layout::FrameError::DimensionsOverflow { have: data.len() }.into()),
    };
    let stride = width
        .checked_mul(bpp)
        .ok_or(crate::layout::FrameError::DimensionsOverflow { have: data.len() })?;
    crate::layout::FrameLayout::checked(
        nerust_render_traits::logical::LogicalSize { width, height },
        bpp,
        stride,
        data.len(),
        true,
    )?;
    prepare_cpu_unpack()?;
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(unsafe {
        tex_image_2d(
            gl::TEXTURE_2D,
            0,
            internal_fmt,
            checked_i32(width)?,
            checked_i32(height)?,
            0,
            data_fmt,
            gl::UNSIGNED_BYTE,
            data.as_ptr().cast(),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pack_ntsc_bytes;
    use crate::upload::StagingBuf;

    #[test]
    fn ntsc_pack_copies_bytes_verbatim_with_zero_pad() {
        // Three full words: verbatim copy, tail zero-padded. A short
        // trailing partial word never copies (offset guard).
        let be: Vec<u8> = (1u8..=12).collect();
        let mut scratch = StagingBuf::new();
        let (height, need) = pack_ntsc_bytes(2, 2, &be, &mut scratch).unwrap();
        assert_eq!((height, need), (2, 16));
        let staged = scratch.staging(need);
        assert_eq!(&staged[..12], &(1u8..=12).collect::<Vec<_>>()[..]);
        assert_eq!(&staged[12..16], &[0, 0, 0, 0]);
    }

    #[test]
    fn ntsc_upload_bytes_match_cpu_order() {
        // Locks the canonical order: GPU upload bytes equal the CPU
        // indexer order byte-for-byte.
        let word = u32::from_be_bytes([0x11, 0x22, 0x33, 0x44]);
        assert_eq!(word.to_be_bytes(), [0x11, 0x22, 0x33, 0x44]);
        let be = word.to_be_bytes();
        let mut scratch = StagingBuf::new();
        let (_, need) = pack_ntsc_bytes(1, 1, &be, &mut scratch).unwrap();
        assert_eq!(need, 4);
        assert_eq!(&scratch.staging(need)[..need], &be);
        assert_eq!(
            u32::from_be_bytes(scratch.staging(need)[..4].try_into().unwrap()),
            word
        );
    }

    #[test]
    fn ntsc_pack_rejects_overflow() {
        let mut scratch = StagingBuf::new();
        let err = pack_ntsc_bytes(usize::MAX, usize::MAX, &[0; 4], &mut scratch).unwrap_err();
        assert!(err.to_string().contains("overflow"));
    }

    #[test]
    fn ntsc_pack_routes_words_through_rgba_word() {
        // Big-endian words decode to RGBA channels; the packed scratch
        // holds the same bytes the CPU indexer reads.
        use nerust_render_traits::rgb::RgbaWord;
        let be = [0x11u8, 0x22, 0x33, 0x44];
        let mut scratch = StagingBuf::new();
        let (_, need) = pack_ntsc_bytes(1, 1, &be, &mut scratch).unwrap();
        let staged: [u8; 4] = scratch.staging(need)[..4].try_into().unwrap();
        assert_eq!(RgbaWord::from_bytes_be(staged).as_u32(), 0x1122_3344);
    }

    #[test]
    fn ntsc_pack_grows_short_kernels_to_texture_height() {
        // One entry against a height-2 store: present row verbatim,
        // missing row zero-padded.
        let mut scratch = StagingBuf::new();
        let (height, need) =
            pack_ntsc_bytes(1, 2, &[0xAA, 0xBB, 0xCC, 0xDD], &mut scratch).unwrap();
        assert_eq!((height, need), (2, 8));
        let staged = scratch.staging(need);
        assert_eq!(&staged[..4], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(&staged[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn ntsc_pack_empty_source_keeps_requested_height() {
        let mut scratch = StagingBuf::new();
        let (height, need) = pack_ntsc_bytes(2, 2, &[], &mut scratch).unwrap();
        assert_eq!((height, need), (2, 16));
        assert!(scratch.staging(need).iter().all(|&b| b == 0));
    }
}
