//! Frame texture domain: allocation-free initial uploads plus the
//! NTSC kernel path. [`super::view::GlView`] coordinates; this module
//! owns the TexImage calls so the view never host-memsets a null
//! upload.

use gl::types::{GLenum, GLint};
use nerust_glwrap::raw::*;

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
