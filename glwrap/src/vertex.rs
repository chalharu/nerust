// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{gl_error_handle, gl_get_error, Error};
use gl;
use gl::types::{GLenum, GLint, GLsizei, GLuint};
use std;
use std::rc::Rc;

pub struct VertexArray {
    vao: VertexArrayVAO,

    // VBOs must be ref-counted because of the many-to-many relationship with VAOs and VBOs
    vbo_refs: Vec<Rc<VertexBuffer>>,
    // IBOs have a one-to-many relationship with VAOs
    ibo_ref: Option<Rc<IndexBuffer>>,
}

struct VertexArrayVAO {
    id: GLuint,
}

impl Drop for VertexArrayVAO {
    fn drop(&mut self) {
        gl_error_handle(|| unsafe { gl::DeleteVertexArrays(1, &self.id) })
            .unwrap_or_else(|x| warn!("{}", x));
    }
}

impl VertexArray {
    pub fn new<F>(cb: F) -> Result<VertexArray, Error>
    where
        F: FnOnce(&mut VertexArrayInitContext) -> Result<(), Error>,
    {
        let mut vao = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
        }
        gl_get_error()?;
        unsafe {
            gl::BindVertexArray(vao);
        }
        gl_get_error()?;

        let mut va = VertexArray {
            vao: VertexArrayVAO { id: vao },
            vbo_refs: Vec::new(),
            ibo_ref: None,
        };

        {
            let mut ctx = VertexArrayInitContext { va: &mut va };
            cb(&mut ctx)?;
        }

        Ok(va)
    }

    pub fn bind_vao<F>(&self, cb: F) -> Result<(), Error>
    where
        F: FnOnce(&VertexArrayContext) -> Result<(), Error>,
    {
        gl_error_handle(|| unsafe { gl::BindVertexArray(self.vao.id) })?;
        gl_get_error()?;
        let ctx = VertexArrayContext { va: self };
        cb(&ctx)?;
        Ok(())
    }

    fn add_vbo(&mut self, vbo: Rc<VertexBuffer>) {
        self.vbo_refs.push(vbo);
    }
    fn set_ibo(&mut self, ibo: Rc<IndexBuffer>) {
        self.ibo_ref = Some(ibo);
    }
}

pub struct VertexArrayInitContext<'a> {
    va: &'a mut VertexArray,
}
impl<'a> VertexArrayInitContext<'a> {
    pub fn bind_vbo<F>(&mut self, vbo: Rc<VertexBuffer>, cb: F) -> Result<(), Error>
    where
        F: FnOnce(VertexArrayBufferContext) -> Result<(), Error>,
    {
        gl_error_handle(|| unsafe { gl::BindBuffer(gl::ARRAY_BUFFER, vbo.id) })?;
        self.va.add_vbo(vbo);
        cb(VertexArrayBufferContext)?;
        Ok(())
    }
    pub fn bind_ibo(&mut self, ibo: Rc<IndexBuffer>) -> Result<(), Error> {
        gl_error_handle(|| unsafe { gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ibo.id) })?;
        self.va.set_ibo(ibo);
        Ok(())
    }
}

pub struct VertexArrayContext<'a> {
    va: &'a VertexArray,
}
impl<'a> VertexArrayContext<'a> {
    pub fn draw_arrays(&self, mode: GLuint, first: GLint, count: GLsizei) -> Result<(), Error> {
        gl_error_handle(|| unsafe { gl::DrawArrays(mode, first, count) })
    }
    pub fn draw_elements(&self, mode: GLuint, count: GLsizei, offset: usize) -> Result<(), Error> {
        let data_type = self
            .va
            .ibo_ref
            .as_ref()
            .expect("No IBO is bound to the VAO")
            .data_type;
        gl_error_handle(|| unsafe {
            gl::DrawElements(
                mode,
                count,
                data_type,
                std::ptr::null::<std::ffi::c_void>().offset(offset as isize),
            )
        })
    }
}

pub struct VertexArrayBufferContext;
impl VertexArrayBufferContext {
    pub fn attr_pointer(
        &self,
        a: Attrib,
        data_size: GLint,
        data_type: GLuint,
        stride: GLsizei,
        offset: usize,
    ) -> Result<(), Error> {
        gl_error_handle(|| unsafe { gl::EnableVertexAttribArray(a.id) })?;
        gl_error_handle(|| unsafe {
            gl::VertexAttribPointer(
                a.id,
                data_size,
                data_type,
                gl::FALSE,
                stride,
                std::ptr::null::<std::ffi::c_void>().offset(offset as isize),
            )
        })
    }
}

pub struct VertexBuffer {
    pub id: GLuint,
}

impl Drop for VertexBuffer {
    fn drop(&mut self) {
        gl_error_handle(|| unsafe { gl::DeleteBuffers(1, &self.id) })
            .unwrap_or_else(|x| warn!("{}", x));
    }
}

impl VertexBuffer {
    pub fn from_slice<T>(s: &[T]) -> Result<VertexBuffer, Error> {
        let mut vbo = 0;
        unsafe { gl::GenBuffers(1, &mut vbo) };
        gl_get_error()?;
        unsafe { gl::BindBuffer(gl::ARRAY_BUFFER, vbo) };
        gl_get_error()?;
        unsafe {
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (s.len() * std::mem::size_of::<T>()) as gl::types::GLsizeiptr,
                std::mem::transmute(s.as_ptr()),
                gl::STATIC_DRAW,
            );
        }
        gl_get_error()?;

        Ok(VertexBuffer { id: vbo })
    }
}

pub struct IndexBuffer {
    id: GLuint,
    data_type: GLenum,
}

impl Drop for IndexBuffer {
    fn drop(&mut self) {
        gl_error_handle(|| unsafe { gl::DeleteBuffers(1, &self.id) })
            .unwrap_or_else(|x| warn!("{}", x));
    }
}

impl IndexBuffer {
    pub fn from_slice<T>(s: &[T], data_type: GLenum) -> Result<IndexBuffer, Error> {
        let mut vbo = 0;
        unsafe {
            gl::GenBuffers(1, &mut vbo);
        }
        gl_get_error()?;
        unsafe {
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, vbo);
        }
        gl_get_error()?;
        unsafe {
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (s.len() * std::mem::size_of::<T>()) as gl::types::GLsizeiptr,
                std::mem::transmute(s.as_ptr()),
                gl::STATIC_DRAW,
            );
        }
        gl_get_error()?;

        Ok(IndexBuffer {
            id: vbo,
            data_type: data_type,
        })
    }
}

#[derive(Copy, Clone)]
pub struct Attrib {
    pub id: GLuint,
}
