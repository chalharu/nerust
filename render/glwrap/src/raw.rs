#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "this module intentionally mirrors pointer-based OpenGL C APIs"
)]

use std::{ffi::CStr, os::raw::c_void};

use gl::types::{
    GLbitfield, GLboolean, GLchar, GLenum, GLfloat, GLint, GLsizei, GLsizeiptr, GLuint,
};

use super::{Error, error::ErrorKind, gl_error_handle};

pub fn get_programiv(program: GLuint, pname: GLenum, params: *mut GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GetProgramiv(program, pname, params) })
}

pub fn get_active_attrib(
    program: GLuint,
    index: GLuint,
    buf_size: GLsizei,
    length: *mut GLsizei,
    size: *mut GLint,
    type_: *mut GLenum,
    name: *mut GLchar,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::GetActiveAttrib(program, index, buf_size, length, size, type_, name)
    })
}

pub fn get_active_uniform(
    program: GLuint,
    index: GLuint,
    buf_size: GLsizei,
    length: *mut GLsizei,
    size: *mut GLint,
    type_: *mut GLenum,
    name: *mut GLchar,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::GetActiveUniform(program, index, buf_size, length, size, type_, name)
    })
}

pub fn get_attrib_location(program: GLuint, name: &CStr) -> Result<GLint, Error> {
    gl_error_handle(|| unsafe { gl::GetAttribLocation(program, name.as_ptr()) })
}

pub fn get_uniform_location(program: GLuint, name: &CStr) -> Result<GLint, Error> {
    gl_error_handle(|| unsafe { gl::GetUniformLocation(program, name.as_ptr()) })
}

pub fn gen_textures(n: GLsizei, textures: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenTextures(n, textures) })
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
pub fn tex_image_2d(
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

pub fn gen_buffers(n: GLsizei, buffers: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenBuffers(n, buffers) })
}

pub fn bind_buffer(target: GLenum, buffer: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindBuffer(target, buffer) })
}

pub fn delete_textures(n: GLsizei, textures: *const GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteTextures(n, textures) })
}

pub fn delete_buffers(n: GLsizei, buffers: *const GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteBuffers(n, buffers) })
}

/// Delete a program name (symmetric with the other raw wrappers).
/// Call only with a current context; best-effort at teardown.
pub fn delete_program(program: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteProgram(program) })
}

pub fn buffer_data(
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
    value: *const GLfloat,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::UniformMatrix4fv(location, count, transpose, value) })
}

pub fn vertex_attrib_pointer(
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
pub fn tex_sub_image_2d(
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

pub fn draw_arrays(mode: GLenum, first: GLint, count: GLsizei) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DrawArrays(mode, first, count) })
}

pub fn draw_elements(
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    offset: usize,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe {
        gl::DrawElements(
            mode,
            count,
            type_,
            // SAFETY: null base plus a byte offset is the standard
            // client-side index address form for `DrawElements`.
            std::ptr::null::<std::ffi::c_void>().add(offset),
        )
    })
}

pub fn gen_vertex_arrays(n: GLsizei, arrays: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenVertexArrays(n, arrays) })
}

pub fn bind_vertex_array(array: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindVertexArray(array) })
}

pub fn delete_vertex_arrays(n: GLsizei, arrays: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteVertexArrays(n, arrays) })
}

pub fn viewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Viewport(x, y, width, height) })
}

/// Read back the current viewport box `[x, y, width, height]`.
/// Callers save this around composites that temporarily widen the
/// viewport (e.g. the full-surface OSD blit) and restore it after.
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
    let mut value: GLint = 0;
    gl_error_handle(|| unsafe {
        gl::GetIntegerv(pname, &mut value);
        value
    })
}

/// Checked `u32` variant for GL names (program, VAO, buffer, blend
/// factors): negative values refuse as `InvalidValue` instead of
/// wrapping through `as u32` at the restore site.
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

pub fn get_tex_image(
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
