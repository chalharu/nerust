// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod error;
mod raw;
mod vertex;

use self::error::*;
use gl::types::{GLchar, GLenum, GLint, GLsizei, GLuint};
pub use raw::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::{ptr, str};
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
    uniforms: HashMap<String, GLint>,
}

impl Shader {
    pub fn new(vert_src: &str, flag_src: &str) -> Self {
        Self::try_new(vert_src, flag_src).unwrap_or_else(|e| panic!("{e}"))
    }

    pub fn try_new(vert_src: &str, flag_src: &str) -> Result<Self, String> {
        let vtx_shader_id = compile_shader(vert_src, gl::VERTEX_SHADER)?;
        let flag_shader_id = compile_shader(flag_src, gl::FRAGMENT_SHADER)?;
        let program_id = link_program(vtx_shader_id, flag_shader_id)?;

        Ok(Self {
            program: program_id,
            attributes: get_attributes(program_id),
            uniforms: get_uniforms(program_id),
        })
    }

    pub fn get_attribute(&self, key: &str) -> GLuint {
        self.attributes.get(key).map_or_else(|| 0, |&x| x)
    }

    pub fn get_uniform(&self, key: &str) -> GLint {
        self.uniforms.get(key).map_or(-1, |&x| x)
    }

    pub fn use_program(&self) {
        use_program(self.program).unwrap();
    }
}

fn get_attributes(program_id: GLuint) -> HashMap<String, GLuint> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_ATTRIBUTES, &mut count).unwrap();

    let mut size: GLint = 0;
    let mut ty: GLenum = 0;
    const BUF_SIZE: GLsizei = 16;
    let mut name_buf = vec![GLchar::default(); BUF_SIZE as usize];
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
            name_buf.as_mut_ptr(),
        )
        .unwrap();
        let name = String::from(
            unsafe { CStr::from_ptr(name_buf.as_ptr()) }
                .to_str()
                .unwrap(),
        );
        let location =
            get_attrib_location(program_id, unsafe { CStr::from_ptr(name_buf.as_ptr()) }).unwrap();
        let _ = result.insert(
            name,
            u32::try_from(location).expect("OpenGL attribute location must be non-negative"),
        );
    }
    result
}

fn get_uniforms(program_id: GLuint) -> HashMap<String, GLint> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_UNIFORMS, &mut count).unwrap();

    let mut size: GLint = 0;
    let mut ty: GLenum = 0;
    const BUF_SIZE: GLsizei = 16;
    let mut name_buf = vec![GLchar::default(); BUF_SIZE as usize];
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
            name_buf.as_mut_ptr(),
        )
        .unwrap();
        let name = String::from(
            unsafe { CStr::from_ptr(name_buf.as_ptr()) }
                .to_str()
                .unwrap(),
        );
        let location =
            get_uniform_location(program_id, unsafe { CStr::from_ptr(name_buf.as_ptr()) }).unwrap();
        let _ = result.insert(name, location);
    }

    result
}

fn info_log_buffer(len: GLint) -> Vec<u8> {
    vec![0; usize::try_from(len).expect("OpenGL info log length must be non-negative")]
}

fn trim_info_log(buf: &mut Vec<u8>, written: GLsizei) -> &str {
    let written =
        usize::try_from(written).expect("OpenGL written info log length must be non-negative");
    let written = written.min(buf.len());
    buf.truncate(written);
    if buf.last() == Some(&0) {
        let _ = buf.pop();
    }
    str::from_utf8(buf.as_slice()).expect("OpenGL info log not valid utf8")
}

fn compile_shader(src: &str, ty: GLenum) -> Result<GLuint, String> {
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
            let mut written = 0;
            let mut buf = info_log_buffer(len);
            gl::GetShaderInfoLog(shader, len, &mut written, buf.as_mut_ptr() as *mut GLchar);
            gl::DeleteShader(shader);
            return Err(trim_info_log(&mut buf, written).to_owned());
        }
    }
    Ok(shader)
}

fn link_program(vs: GLuint, fs: GLuint) -> Result<GLuint, String> {
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
            let mut written = 0;
            let mut buf = info_log_buffer(len);
            gl::GetProgramInfoLog(program, len, &mut written, buf.as_mut_ptr() as *mut GLchar);
            gl::DeleteProgram(program);
            return Err(trim_info_log(&mut buf, written).to_owned());
        }
        Ok(program)
    }
}
