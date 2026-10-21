//! Low-level calls. Pointer-bearing functions require the caller to uphold GL memory rules.
use crate::bindings as gl;

use std::{ffi::CStr, os::raw::c_void};

use gl::types::{
    GLbitfield, GLboolean, GLchar, GLenum, GLfloat, GLint, GLsizei, GLsizeiptr, GLuint,
};

use super::{Error, error::ErrorKind, gl_error_handle};

pub fn get_programiv(program: GLuint, pname: GLenum, params: &mut GLint) -> Result<(), Error> {
    if !matches!(
        pname,
        gl::LINK_STATUS
            | gl::INFO_LOG_LENGTH
            | gl::ACTIVE_ATTRIBUTES
            | gl::ACTIVE_ATTRIBUTE_MAX_LENGTH
            | gl::ACTIVE_UNIFORMS
            | gl::ACTIVE_UNIFORM_MAX_LENGTH
    ) {
        return Err(Error::new(vec![ErrorKind::InvalidEnum]));
    }
    gl_error_handle(|| unsafe { gl::GetProgramiv(program, pname, params) })
}

pub fn get_active_attrib(
    program: GLuint,
    index: GLuint,
    buf_size: GLsizei,
    length: &mut GLsizei,
    size: &mut GLint,
    type_: &mut GLenum,
    name: &mut [GLchar],
) -> Result<(), Error> {
    validate_count(buf_size, name.len())?;
    gl_error_handle(|| unsafe {
        gl::GetActiveAttrib(
            program,
            index,
            buf_size,
            length,
            size,
            type_,
            name.as_mut_ptr(),
        )
    })
}

pub fn get_active_uniform(
    program: GLuint,
    index: GLuint,
    buf_size: GLsizei,
    length: &mut GLsizei,
    size: &mut GLint,
    type_: &mut GLenum,
    name: &mut [GLchar],
) -> Result<(), Error> {
    validate_count(buf_size, name.len())?;
    gl_error_handle(|| unsafe {
        gl::GetActiveUniform(
            program,
            index,
            buf_size,
            length,
            size,
            type_,
            name.as_mut_ptr(),
        )
    })
}

pub fn get_attrib_location(program: GLuint, name: &CStr) -> Result<GLint, Error> {
    gl_error_handle(|| unsafe { gl::GetAttribLocation(program, name.as_ptr()) })
}

pub fn get_uniform_location(program: GLuint, name: &CStr) -> Result<GLint, Error> {
    gl_error_handle(|| unsafe { gl::GetUniformLocation(program, name.as_ptr()) })
}

fn validate_count(n: GLsizei, len: usize) -> Result<(), Error> {
    if usize::try_from(n).is_ok_and(|n| n <= len) {
        Ok(())
    } else {
        Err(Error::new(vec![ErrorKind::InvalidValue]))
    }
}

pub fn gen_textures(n: GLsizei, textures: &mut [GLuint]) -> Result<(), Error> {
    validate_count(n, textures.len())?;
    gl_error_handle(|| unsafe { gl::GenTextures(n, textures.as_mut_ptr()) })
}

pub fn pixel_storei(pname: GLenum, param: GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::PixelStorei(pname, param) })
}

pub fn bind_texture(target: GLenum, texture: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindTexture(target, texture) })
}

pub fn active_texture(texture: GLenum) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::ActiveTexture(texture) })
}

#[expect(
    clippy::too_many_arguments,
    reason = "OpenGL texture upload parameters map directly to the C API"
)]
/// # Safety
/// `pixels` must satisfy GL's format, extent, pixel-store and PBO memory requirements.
pub unsafe fn tex_image_2d(
    target: GLenum,
    level: GLint,
    internalformat: GLint,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::TexImage2D(
            target,
            level,
            internalformat,
            width,
            height,
            border,
            format,
            type_,
            pixels,
        )
    })
}

pub fn tex_parameteri(target: GLenum, pname: GLenum, param: GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::TexParameteri(target, pname, param) })
}

pub fn gen_buffers(n: GLsizei, buffers: &mut [GLuint]) -> Result<(), Error> {
    validate_count(n, buffers.len())?;
    gl_error_handle(|| unsafe { gl::GenBuffers(n, buffers.as_mut_ptr()) })
}

pub fn bind_buffer(target: GLenum, buffer: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindBuffer(target, buffer) })
}

pub fn delete_textures(n: GLsizei, textures: &[GLuint]) -> Result<(), Error> {
    validate_count(n, textures.len())?;
    gl_error_handle(|| unsafe { gl::DeleteTextures(n, textures.as_ptr()) })?;
    super::check_frame()
}

pub fn delete_buffers(n: GLsizei, buffers: &[GLuint]) -> Result<(), Error> {
    validate_count(n, buffers.len())?;
    gl_error_handle(|| unsafe { gl::DeleteBuffers(n, buffers.as_ptr()) })?;
    super::check_frame()
}

/// Delete a program name (symmetric with the other raw wrappers).
/// Call only with a current context; best-effort at teardown.
pub fn delete_program(program: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteProgram(program) })?;
    super::check_frame()
}

/// # Safety
/// `data` must address `size` initialized bytes, or be null to allocate storage.
pub unsafe fn buffer_data(
    target: GLenum,
    size: GLsizeiptr,
    data: *const c_void,
    usage: GLenum,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BufferData(target, size, data, usage) })
}

pub fn clear_color(
    red: GLfloat,
    green: GLfloat,
    blue: GLfloat,
    alpha: GLfloat,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::ClearColor(red, green, blue, alpha) })
}

pub fn clear(mask: GLbitfield) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Clear(mask) })
}

pub fn enable_vertex_attrib_array(index: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::EnableVertexAttribArray(index) })
}

pub fn uniform_1i(location: GLint, v0: GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Uniform1i(location, v0) })
}

pub fn uniform_2f(location: GLint, v0: GLfloat, v1: GLfloat) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Uniform2f(location, v0, v1) })
}

pub fn uniform_matrix_4fv(
    location: GLint,
    count: GLsizei,
    transpose: GLboolean,
    value: &[GLfloat; 16],
) -> Result<(), Error> {
    if count != 1 {
        return Err(Error::new(vec![ErrorKind::InvalidValue]));
    }
    gl_error_handle(|| unsafe { gl::UniformMatrix4fv(location, count, transpose, value.as_ptr()) })
}

/// # Safety
/// The pointer and bound buffer must remain valid for all subsequent draws.
pub unsafe fn vertex_attrib_pointer(
    index: GLuint,
    size: GLint,
    type_: GLenum,
    normalized: GLboolean,
    stride: GLsizei,
    pointer: *const c_void,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::VertexAttribPointer(index, size, type_, normalized, stride, pointer)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "OpenGL texture upload parameters map directly to the C API"
)]
/// # Safety
/// `pixels` must satisfy GL's format, extent, pixel-store and PBO memory requirements.
pub unsafe fn tex_sub_image_2d(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::TexSubImage2D(
            target, level, xoffset, yoffset, width, height, format, type_, pixels,
        )
    })
}

/// # Safety
/// All enabled vertex arrays must have sufficient backing storage for this draw.
pub unsafe fn draw_arrays(mode: GLenum, first: GLint, count: GLsizei) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DrawArrays(mode, first, count) })
}

/// Buffer-object byte offset as a GL client address; `u32`-bounded, no provenance.
pub(crate) fn index_offset(offset: u32) -> *const std::ffi::c_void {
    std::ptr::without_provenance::<std::ffi::c_void>(offset as usize)
}

/// # Safety
/// Index and vertex storage must cover every element referenced by this draw.
pub unsafe fn draw_elements(
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    offset: u32,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::DrawElements(
            mode,
            count,
            type_,
            // Index-buffer byte offset as an address without provenance.
            // `u32` bounds the offset; the `usize` widening cannot overflow.
            index_offset(offset),
        )
    })
}

pub fn gen_vertex_arrays(n: GLsizei, arrays: &mut [GLuint]) -> Result<(), Error> {
    validate_count(n, arrays.len())?;
    gl_error_handle(|| unsafe { gl::GenVertexArrays(n, arrays.as_mut_ptr()) })
}

pub fn bind_vertex_array(array: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindVertexArray(array) })
}

pub fn delete_vertex_arrays(n: GLsizei, arrays: &[GLuint]) -> Result<(), Error> {
    validate_count(n, arrays.len())?;
    gl_error_handle(|| unsafe { gl::DeleteVertexArrays(n, arrays.as_ptr()) })?;
    super::check_frame()
}

pub fn viewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Viewport(x, y, width, height) })
}

/// Read back the current viewport box `[x, y, width, height]`.
/// Callers save/restore this around viewport-widening composites.
pub fn get_viewport(viewport: &mut [GLint; 4]) -> Result<(), Error> {
    let out = viewport.as_mut_ptr();
    gl_error_handle(|| unsafe { gl::GetIntegerv(gl::VIEWPORT, out) })
}

pub fn enable(cap: GLenum) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Enable(cap) })
}

pub fn disable(cap: GLenum) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Disable(cap) })
}

/// `glIsEnabled` with error drain (debug) / fast path (release).
pub fn is_enabled(cap: GLenum) -> Result<bool, Error> {
    gl_error_handle(|| unsafe { gl::IsEnabled(cap) == gl::TRUE })
}

/// Single-integer `glGetIntegerv` for save/restore guards
/// (program, active texture, blend factors, bindings).
pub fn get_integer(pname: GLenum) -> Result<GLint, Error> {
    // Never pass an unclassified query to a one-element stack allocation.
    if !matches!(
        pname,
        gl::CURRENT_PROGRAM
            | gl::ACTIVE_TEXTURE
            | gl::VERTEX_ARRAY_BINDING
            | gl::ARRAY_BUFFER_BINDING
            | gl::BLEND_SRC_RGB
            | gl::BLEND_DST_RGB
            | gl::BLEND_SRC_ALPHA
            | gl::BLEND_DST_ALPHA
            | gl::UNPACK_ALIGNMENT
            | gl::UNPACK_ROW_LENGTH
            | gl::MAX_TEXTURE_SIZE
    ) {
        return Err(Error::new(vec![ErrorKind::InvalidEnum]));
    }
    let mut value: GLint = 0;
    gl_error_handle(|| unsafe {
        gl::GetIntegerv(pname, &mut value);
        value
    })
}

/// Checked `u32` variant for GL names (program, VAO, buffer, blend factors).
/// Negatives refuse as `InvalidValue` instead of wrapping through `as u32`.
pub fn get_integer_u32(pname: GLenum) -> Result<GLuint, Error> {
    let value = get_integer(pname)?;
    u32::try_from(value).map_err(|_| Error::new(vec![ErrorKind::InvalidValue]))
}

/// Batched per-frame drain. Prefer this over per-call drains in
/// release builds; wrappers are no-ops there (see `gl_error_handle`).
pub fn check_frame() -> Result<(), Error> {
    super::check_frame()
}

pub fn blend_func(sfactor: GLenum, dfactor: GLenum) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BlendFunc(sfactor, dfactor) })
}

pub fn blend_func_separate(
    src_rgb: GLuint,
    dst_rgb: GLuint,
    src_alpha: GLuint,
    dst_alpha: GLuint,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BlendFuncSeparate(src_rgb, dst_rgb, src_alpha, dst_alpha) })
}

pub fn use_program(program: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::UseProgram(program) })
}

/// # Safety
/// The output must cover the complete texture using the current pack/PBO state.
pub unsafe fn get_tex_image(
    target: GLenum,
    level: GLint,
    format: GLenum,
    type_: GLenum,
    pixels: *mut std::os::raw::c_void,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GetTexImage(target, level, format, type_, pixels) })
}

pub fn gl_string(name: u32) -> Option<String> {
    let value = unsafe { gl::GetString(name) };
    if value.is_null() {
        return None;
    }

    Some(
        unsafe { CStr::from_ptr(value.cast()) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::error::{Error, ErrorKind};
    use crate::bindings as gl;

    #[test]
    fn rejects_vector_queries_and_short_output_slices_before_ffi() {
        assert!(super::get_integer(gl::VIEWPORT).is_err());
        assert!(super::get_integer(gl::MAX_VIEWPORT_DIMS).is_err());
        assert!(super::get_integer(u32::MAX).is_err());
        assert!(super::gen_textures(2, &mut [0]).is_err());
        assert!(super::gen_buffers(-1, &mut []).is_err());
        assert!(super::gen_vertex_arrays(1, &mut []).is_err());
        assert!(super::delete_textures(2, &[0]).is_err());
        assert!(super::get_programiv(0, gl::COMPUTE_WORK_GROUP_SIZE, &mut 0).is_err());
        assert!(super::uniform_matrix_4fv(0, 2, gl::FALSE, &[0.0; 16]).is_err());
    }

    #[test]
    fn index_offset_carries_no_provenance() {
        // `u32` offsets widen exactly; the address equals the offset value.
        assert_eq!(super::index_offset(0) as usize, 0);
        assert_eq!(super::index_offset(8) as usize, 8);
        assert_eq!(super::index_offset(u32::MAX) as usize, u32::MAX as usize);
    }

    #[test]
    fn error_kind_maps_known_codes() {
        assert_eq!(ErrorKind::from(gl::INVALID_ENUM), ErrorKind::InvalidEnum);
        assert_eq!(ErrorKind::from(gl::INVALID_VALUE), ErrorKind::InvalidValue);
        assert_eq!(
            ErrorKind::from(gl::INVALID_OPERATION),
            ErrorKind::InvalidOperation
        );
        assert_eq!(
            ErrorKind::from(gl::INVALID_FRAMEBUFFER_OPERATION),
            ErrorKind::InvalidFramebufferOperation
        );
        assert_eq!(ErrorKind::from(0xDEAD), ErrorKind::Unexpected(0xDEAD));
    }

    #[test]
    fn error_display_joins_kinds_with_newlines() {
        let err = Error::new(vec![ErrorKind::InvalidValue, ErrorKind::OutOfMemory]);
        let text = err.to_string();
        assert!(text.contains('\n'));
        assert!(text.contains("legal value"));
        let empty = Error::new(vec![]);
        assert_eq!(empty.to_string(), "");
        let round: Error = vec![ErrorKind::ContextLost].into();
        assert!(round.to_string().contains("lost"));
    }
}
