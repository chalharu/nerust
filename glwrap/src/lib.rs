// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(
    clippy::manual_slice_size_calculation,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::single_component_path_imports,
    clippy::too_many_arguments,
    clippy::uninit_vec
)]

mod error;
mod raw;
mod vertex;

use self::error::*;
use gl::types::{GLchar, GLenum, GLint, GLsizei, GLuint};
pub use raw::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::{ptr, slice, str};
pub use vertex::*;

fn gl_error_handle<T, F: Fn() -> T>(func: F) -> Result<T, Error> {
    let result = func();
    gl_get_error()?;
    Ok(result)
}

fn gl_get_error() -> Result<(), Error> {
    let mut error_code = unsafe { gl::GetError() };

    if error_code != gl::NO_ERROR {
        let mut error = Vec::new();
        loop {
            error.push(ErrorKind::from(error_code));
            error_code = unsafe { gl::GetError() };
            if error_code == gl::NO_ERROR {
                return Err(Error::from(error));
            }
        }
    }
    Ok(())
}

pub fn use_program(program: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::UseProgram(program) })
}

#[derive(Debug)]
pub struct Shader {
    program: GLuint,
    attributes: HashMap<String, GLuint>,
    uniforms: HashMap<String, GLuint>,
}

impl Shader {
    pub fn new(vert_src: &str, flag_src: &str) -> Self {
        let vtx_shader_id = compile_shader(vert_src, gl::VERTEX_SHADER);
        let flag_shader_id = compile_shader(flag_src, gl::FRAGMENT_SHADER);
        let program_id = link_program(vtx_shader_id, flag_shader_id);

        Self {
            program: program_id,
            attributes: get_attributes(program_id),
            uniforms: get_uniforms(program_id),
        }
    }

    pub fn get_attribute(&self, key: &str) -> GLuint {
        self.attributes.get(key).map_or_else(|| 0, |&x| x)
    }

    pub fn get_uniform(&self, key: &str) -> GLuint {
        self.uniforms.get(key).map_or_else(|| 0, |&x| x)
    }

    pub fn use_program(&self) {
        use_program(self.program).unwrap();
    }
}

unsafe fn alloc<T>(len: usize) -> *mut T {
    let mut vec = Vec::<T>::with_capacity(len);
    vec.set_len(len);
    Box::into_raw(vec.into_boxed_slice()) as *mut T
}

unsafe fn free<T>(raw: *mut T, len: usize) {
    let s = slice::from_raw_parts_mut(raw, len);
    let _ = Box::from_raw(s);
}

fn get_attributes(program_id: GLuint) -> HashMap<String, GLuint> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_ATTRIBUTES, &mut count).unwrap();

    let mut size: GLint = 0;
    let mut ty: GLenum = 0;
    const BUF_SIZE: GLsizei = 16;
    let name_buf = unsafe { alloc::<GLchar>(BUF_SIZE as usize) };
    let mut length: GLsizei = 0;

    let mut result = HashMap::new();

    for i in 0..count as GLuint {
        get_active_attrib(
            program_id,
            i,
            BUF_SIZE,
            &mut length,
            &mut size,
            &mut ty,
            name_buf,
        )
        .unwrap();
        let name = String::from(unsafe { CStr::from_ptr(name_buf) }.to_str().unwrap());
        let _ = result.insert(name, i);
    }
    unsafe { free(name_buf, BUF_SIZE as usize) };
    result
}

fn get_uniforms(program_id: GLuint) -> HashMap<String, GLuint> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_UNIFORMS, &mut count).unwrap();

    let mut size: GLint = 0;
    let mut ty: GLenum = 0;
    const BUF_SIZE: GLsizei = 16;
    let name_buf = unsafe { alloc::<GLchar>(BUF_SIZE as usize) };
    let mut length: GLsizei = 0;

    let mut result = HashMap::new();

    for i in 0..count as GLuint {
        get_active_uniform(
            program_id,
            i,
            BUF_SIZE,
            &mut length,
            &mut size,
            &mut ty,
            name_buf,
        )
        .unwrap();
        let name = String::from(unsafe { CStr::from_ptr(name_buf) }.to_str().unwrap());
        let _ = result.insert(name, i);
    }

    unsafe { free(name_buf, BUF_SIZE as usize) };
    result
}

fn compile_shader(src: &str, ty: GLenum) -> GLuint {
    let shader = unsafe { gl::CreateShader(ty) };
    let c_str = CString::new(src.as_bytes()).unwrap();
    unsafe {
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), ptr::null());
        gl::CompileShader(shader);

        let mut status = GLint::from(gl::FALSE);
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut status);

        if status != GLint::from(gl::TRUE) {
            let mut len = 0;
            gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
            let mut buf = Vec::with_capacity(len as usize);
            buf.set_len((len as usize) - 1); // subtract 1 to skip the trailing null character
            gl::GetShaderInfoLog(
                shader,
                len,
                ptr::null_mut(),
                buf.as_mut_ptr() as *mut GLchar,
            );
            panic!(
                "{}",
                str::from_utf8(&buf).expect("ShaderInfoLog not valid utf8")
            );
        }
    }
    shader
}

fn link_program(vs: GLuint, fs: GLuint) -> GLuint {
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vs);
        gl::AttachShader(program, fs);
        gl::LinkProgram(program);

        let mut status = GLint::from(gl::FALSE);
        get_programiv(program, gl::LINK_STATUS, &mut status).unwrap();

        if status != GLint::from(gl::TRUE) {
            let mut len: GLint = 0;
            get_programiv(program, gl::INFO_LOG_LENGTH, &mut len).unwrap();
            let mut buf = Vec::with_capacity(len as usize);
            buf.set_len((len as usize) - 1); // subtract 1 to skip the trailing null character
            gl::GetProgramInfoLog(
                program,
                len,
                ptr::null_mut(),
                buf.as_mut_ptr() as *mut GLchar,
            );
            panic!(
                "{}",
                str::from_utf8(&buf).expect("ProgramInfoLog not valid utf8")
            );
        }
        program
    }
}
