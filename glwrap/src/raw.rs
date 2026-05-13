// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "this module intentionally mirrors pointer-based OpenGL C APIs"
)]

use super::{Error, gl_error_handle};
use gl::types::{
    GLbitfield, GLboolean, GLchar, GLenum, GLfloat, GLint, GLsizei, GLsizeiptr, GLuint,
};
use std::os::raw::c_void;

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

pub fn gen_textures(n: GLsizei, textures: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenTextures(n, textures) })
}

pub fn pixel_storei(pname: GLenum, param: GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::PixelStorei(pname, param) })
}

pub fn bind_texture(target: GLenum, texture: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindTexture(target, texture) })
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

pub fn buffer_data(
    target: GLenum,
    size: GLsizeiptr,
    data: *const c_void,
    usage: GLenum,
) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BufferData(target, size, data, usage) })
}

// pub fn clear_color(
//     red: GLfloat,
//     green: GLfloat,
//     blue: GLfloat,
//     alpha: GLfloat,
// ) -> Result<(), Error> {
//     gl_error_handle(|| unsafe { gl::ClearColor(red, green, blue, alpha) })
// }

// pub fn clear_depth(depth: GLdouble) -> Result<(), Error> {
//     gl_error_handle(|| unsafe { gl::ClearDepth(depth) })
// }

pub fn clear(mask: GLbitfield) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Clear(mask) })
}

pub fn enable_vertex_attrib_array(index: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::EnableVertexAttribArray(index) })
}

pub fn uniform_1i(location: GLint, v0: GLint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Uniform1i(location, v0) })
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

pub fn gen_vertex_arrays(n: GLsizei, arrays: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenVertexArrays(n, arrays) })
}

pub fn bind_vertex_array(array: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::BindVertexArray(array) })
}

pub fn delete_vertex_arrays(n: GLsizei, arrays: *mut GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::DeleteVertexArrays(n, arrays) })
}

pub fn generate_mipmap(target: GLenum) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::GenerateMipmap(target) })
}

pub fn viewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::Viewport(x, y, width, height) })
}
