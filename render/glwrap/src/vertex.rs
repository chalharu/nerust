use std::rc::Rc;

use gl::types::{GLenum, GLint, GLsizei, GLuint};

use super::{
    CurrentContext, Error,
    raw::{
        bind_buffer, bind_vertex_array, buffer_data, delete_buffers, delete_vertex_arrays,
        draw_arrays, enable_vertex_attrib_array, gen_buffers, gen_vertex_arrays,
        vertex_attrib_pointer,
    },
};

/// GL vertex array plus the buffers it references. Single-threaded by
/// construction: `Rc`/`Cell` internals are `!Send`, so all names live
/// and die on the GL context thread. Teardown deletes held buffers
/// first, then the VAO that references them (see
/// [`VertexArray::destroy_if_current`]); the view teardown follows the
/// same leaf-first order.
#[derive(Debug)]
pub struct VertexArray {
    vao: VertexArrayVAO,

    // VBOs must be ref-counted because of the many-to-many relationship with VAOs and VBOs
    vbo_refs: Vec<Rc<VertexBuffer>>,
    // IBOs have a one-to-many relationship with VAOs
    ibo_ref: Option<Rc<IndexBuffer>>,
}

#[derive(Debug)]
struct VertexArrayVAO {
    id: GLuint,
    deleted: bool,
}

/// No GL calls here: without a current context delete would fail.
impl Drop for VertexArrayVAO {
    fn drop(&mut self) {
        if !self.deleted {
            log::debug!("orphan GL VAO {} leaked at drop", self.id);
        }
    }
}

impl VertexArray {
    pub fn new<F>(cb: F) -> Result<VertexArray, Error>
    where
        F: FnOnce(&mut VertexArrayInitContext<'_>) -> Result<(), Error>,
    {
        let mut vao = 0;
        gen_vertex_arrays(1, &mut vao)?;
        // Scope guard: a failing bind or `cb` must not orphan the VAO.
        // Wrappers already drain per call (debug) / batched (release),
        // so no second `gl_get_error` drain here.
        if let Err(e) = bind_vertex_array(vao) {
            let _ = delete_vertex_arrays(1, &mut vao);
            return Err(e);
        }
        let mut va = VertexArray {
            vao: VertexArrayVAO {
                id: vao,
                deleted: false,
            },
            vbo_refs: Vec::new(),
            ibo_ref: None,
        };

        {
            let mut ctx = VertexArrayInitContext { va: &mut va };
            if let Err(e) = cb(&mut ctx) {
                let _ = delete_vertex_arrays(1, &mut vao);
                return Err(e);
            }
        }

        Ok(va)
    }

    /// Delete the VAO and held buffers when the context is current.
    /// No GL call without a current context; Drop reports the leak.
    pub fn destroy_if_current(&mut self, current: Option<CurrentContext>) -> bool {
        let Some(_current) = current else {
            log::debug!("orphan GL VAO {} skipped: no current context", self.vao.id);
            self.vao.deleted = true;
            self.vbo_refs.clear();
            self.ibo_ref = None;
            return true;
        };
        let mut failed = false;
        for vbo in self.vbo_refs.iter() {
            if vbo.destroy_if_current(Some(_current)) {
                failed = true;
            }
        }
        if let Some(ibo) = self.ibo_ref.as_ref()
            && ibo.destroy_if_current(Some(_current))
        {
            failed = true;
        }
        if !self.vao.deleted {
            use super::raw::delete_vertex_arrays;
            if delete_vertex_arrays(1, &self.vao.id as *const _ as *mut _).is_err() {
                failed = true;
            }
            self.vao.deleted = true;
        }
        self.vbo_refs.clear();
        self.ibo_ref = None;
        failed
    }

    pub fn bind_vao<F>(&self, cb: F) -> Result<(), Error>
    where
        F: FnOnce(&VertexArrayContext<'_>) -> Result<(), Error>,
    {
        bind_vertex_array(self.vao.id)?;
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

#[derive(Debug)]
pub struct VertexArrayInitContext<'a> {
    va: &'a mut VertexArray,
}
impl VertexArrayInitContext<'_> {
    pub fn bind_vbo<F>(&mut self, vbo: Rc<VertexBuffer>, cb: F) -> Result<(), Error>
    where
        F: FnOnce(VertexArrayBufferContext) -> Result<(), Error>,
    {
        bind_buffer(gl::ARRAY_BUFFER, vbo.id)?;
        self.va.add_vbo(vbo);
        cb(VertexArrayBufferContext)?;
        Ok(())
    }
    #[expect(dead_code, reason = "IBO binding for indexed draw future use")]
    fn bind_ibo(&mut self, ibo: Rc<IndexBuffer>) -> Result<(), Error> {
        bind_buffer(gl::ELEMENT_ARRAY_BUFFER, ibo.id)?;
        self.va.set_ibo(ibo);
        Ok(())
    }
}

#[derive(Debug)]
pub struct VertexArrayContext<'a> {
    va: &'a VertexArray,
}
impl VertexArrayContext<'_> {
    pub fn draw_arrays(&self, mode: GLuint, first: GLint, count: GLsizei) -> Result<(), Error> {
        draw_arrays(mode, first, count)
    }
    #[expect(dead_code, reason = "indexed draw for future renderer use")]
    fn draw_elements(&self, mode: GLuint, count: GLsizei, offset: usize) -> Result<(), Error> {
        let data_type = self
            .va
            .ibo_ref
            .as_ref()
            .expect("No IBO is bound to the VAO")
            .data_type;
        super::raw::draw_elements(mode, count, data_type, offset)
    }
}

#[derive(Debug, Clone, Copy)]
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
        enable_vertex_attrib_array(a.id)?;
        vertex_attrib_pointer(
            a.id,
            data_size,
            data_type,
            gl::FALSE,
            stride,
            // SAFETY: null base plus a byte offset is the standard
            // client-side attribute address form for a bound VBO.
            unsafe { std::ptr::null::<std::ffi::c_void>().add(offset) },
        )
    }
}

#[derive(Debug)]
pub struct VertexBuffer {
    pub id: GLuint,
    deleted: std::cell::Cell<bool>,
}

/// No GL calls here: without a current context delete would fail.
impl Drop for VertexBuffer {
    fn drop(&mut self) {
        if !self.deleted.get() {
            log::debug!("orphan GL VBO {} leaked at drop", self.id);
        }
    }
}

impl VertexBuffer {
    pub fn from_slice<T>(s: &[T]) -> Result<VertexBuffer, Error> {
        let mut vbo = 0;
        gen_buffers(1, &mut vbo)?;
        // Scope guard: a failing bind or upload must not orphan `vbo`.
        // Wrappers already drain per call, so no extra `gl_get_error`.
        if let Err(e) = bind_buffer(gl::ARRAY_BUFFER, vbo) {
            let _ = delete_buffers(1, &vbo);
            return Err(e);
        }
        let size = std::mem::size_of_val(s) as gl::types::GLsizeiptr;
        if let Err(e) = buffer_data(gl::ARRAY_BUFFER, size, s.as_ptr() as _, gl::STATIC_DRAW) {
            let _ = delete_buffers(1, &vbo);
            return Err(e);
        }

        Ok(VertexBuffer {
            id: vbo,
            deleted: std::cell::Cell::new(false),
        })
    }

    /// Delete the buffer when the context is current.
    /// `None` skips the GL call and reports failure; Drop logs the leak.
    pub fn destroy_if_current(&self, current: Option<CurrentContext>) -> bool {
        if self.deleted.get() {
            return false;
        }
        let Some(_current) = current else {
            log::debug!("orphan GL VBO {} skipped: no current context", self.id);
            self.deleted.set(true);
            return true;
        };
        let failed = super::raw::delete_buffers(1, &self.id).is_err();
        self.deleted.set(true);
        failed
    }
}

#[derive(Debug)]
pub struct IndexBuffer {
    id: GLuint,

    data_type: GLenum,
    deleted: std::cell::Cell<bool>,
}

/// No GL calls here: without a current context delete would fail.
impl Drop for IndexBuffer {
    fn drop(&mut self) {
        if !self.deleted.get() {
            log::debug!("orphan GL IBO {} leaked at drop", self.id);
        }
    }
}

impl IndexBuffer {
    pub fn from_slice<T>(s: &[T], data_type: GLenum) -> Result<IndexBuffer, Error> {
        let mut vbo = 0;
        gen_buffers(1, &mut vbo)?;
        if let Err(e) = bind_buffer(gl::ELEMENT_ARRAY_BUFFER, vbo) {
            let _ = delete_buffers(1, &vbo);
            return Err(e);
        }
        let size = std::mem::size_of_val(s) as gl::types::GLsizeiptr;
        if let Err(e) = buffer_data(
            gl::ELEMENT_ARRAY_BUFFER,
            size,
            s.as_ptr() as _,
            gl::STATIC_DRAW,
        ) {
            let _ = delete_buffers(1, &vbo);
            return Err(e);
        }

        Ok(IndexBuffer {
            id: vbo,
            data_type,
            deleted: std::cell::Cell::new(false),
        })
    }

    /// Delete the buffer when the context is current.
    /// `None` skips the GL call and reports failure; Drop logs the leak.
    pub fn destroy_if_current(&self, current: Option<CurrentContext>) -> bool {
        if self.deleted.get() {
            return false;
        }
        let Some(_current) = current else {
            log::debug!("orphan GL IBO {} skipped: no current context", self.id);
            self.deleted.set(true);
            return true;
        };
        let failed = super::raw::delete_buffers(1, &self.id).is_err();
        self.deleted.set(true);
        failed
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Attrib {
    pub id: GLuint,
}
