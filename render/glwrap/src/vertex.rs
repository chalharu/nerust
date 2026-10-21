use crate::bindings as gl;
use std::rc::Rc;

use gl::types::{GLenum, GLint, GLsizei, GLuint};

use super::{
    Current, Error, ScopeGuard,
    raw::{
        bind_buffer, bind_vertex_array, buffer_data, delete_buffers, delete_vertex_arrays,
        draw_arrays, enable_vertex_attrib_array, gen_buffers, gen_vertex_arrays,
        vertex_attrib_pointer,
    },
};

/// GL vertex array plus the buffers it references. Single-threaded by
/// construction (`Rc`/`Cell` are `!Send`); teardown is leaf-first.
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

/// Initialization owns the names even while a user callback is unwinding.
struct PendingArray(Option<VertexArray>);

impl Drop for PendingArray {
    fn drop(&mut self) {
        if let Some(array) = self.0.as_mut()
            && array.destroy_if_current(Current::assert_current(true))
        {
            log::debug!("failed to clean up an incomplete GL vertex array");
        }
    }
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
        let generated = gen_vertex_arrays(1, std::slice::from_mut(&mut vao));
        let mut pending = PendingArray(Some(VertexArray {
            vao: VertexArrayVAO {
                id: vao,
                deleted: false,
            },
            vbo_refs: Vec::new(),
            ibo_ref: None,
        }));
        generated?;
        bind_vertex_array(vao)?;

        {
            let mut ctx = VertexArrayInitContext {
                va: pending.0.as_mut().expect("initialization owns the array"),
            };
            cb(&mut ctx)?;
        }

        super::check_frame()?;
        Ok(pending.0.take().expect("initialization owns the array"))
    }

    /// Delete the VAO and held buffers when the context is current.
    /// No GL call without a current context; Drop reports the leak.
    pub fn destroy_if_current(&mut self, current: Option<Current>) -> bool {
        let (arrays, buffers) = self.destroy_report(current);
        arrays + buffers != 0
    }

    /// Failed array and buffer deletions, counted independently.
    pub fn destroy_report(&mut self, current: Option<Current>) -> (usize, usize) {
        let mut arrays = 0;
        let mut buffers = 0;
        if !self.vao.deleted {
            if current.is_none() || delete_vertex_arrays(1, &[self.vao.id]).is_err() {
                arrays += 1;
            }
            self.vao.deleted = true;
        }
        // Release the array first. Only the last reference may retire a buffer.
        while let Some(vbo) = self.vbo_refs.pop() {
            if Rc::strong_count(&vbo) == 1 && vbo.destroy_if_current(current) {
                buffers += 1;
            }
        }
        if let Some(ibo) = self.ibo_ref.take()
            && Rc::strong_count(&ibo) == 1
            && ibo.destroy_if_current(current)
        {
            buffers += 1;
        }
        (arrays, buffers)
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
        let id = vbo.id;
        self.va.add_vbo(vbo);
        bind_buffer(gl::ARRAY_BUFFER, id)?;
        cb(VertexArrayBufferContext)?;
        Ok(())
    }
    #[expect(dead_code, reason = "IBO binding for indexed draw future use")]
    fn bind_ibo(&mut self, ibo: Rc<IndexBuffer>) -> Result<(), Error> {
        let id = ibo.id;
        self.va.set_ibo(ibo);
        bind_buffer(gl::ELEMENT_ARRAY_BUFFER, id)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct VertexArrayContext<'a> {
    va: &'a VertexArray,
}
impl VertexArrayContext<'_> {
    /// # Safety
    /// The enabled arrays must cover every requested vertex.
    pub unsafe fn draw_arrays(
        &self,
        mode: GLuint,
        first: GLint,
        count: GLsizei,
    ) -> Result<(), Error> {
        unsafe { draw_arrays(mode, first, count) }
    }
    #[expect(dead_code, reason = "indexed draw for future renderer use")]
    unsafe fn draw_elements(&self, mode: GLuint, count: GLsizei, offset: u32) -> Result<(), Error> {
        let data_type = self
            .va
            .ibo_ref
            .as_ref()
            .expect("No IBO is bound to the VAO")
            .data_type;
        unsafe { super::raw::draw_elements(mode, count, data_type, offset) }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VertexArrayBufferContext;
impl VertexArrayBufferContext {
    /// # Safety
    /// The bound buffer and attribute layout must cover every subsequent draw.
    pub unsafe fn attr_pointer(
        &self,
        a: Attrib,
        data_size: GLint,
        data_type: GLuint,
        stride: GLsizei,
        offset: u32,
    ) -> Result<(), Error> {
        enable_vertex_attrib_array(a.id)?;
        unsafe {
            vertex_attrib_pointer(
                a.id,
                data_size,
                data_type,
                gl::FALSE,
                stride,
                // Buffer-object byte offset as an address: no provenance,
                // `u32`-bounded so the `usize` widening cannot overflow.
                std::ptr::without_provenance::<std::ffi::c_void>(offset as usize),
            )
        }
    }
}

#[derive(Debug)]
pub struct VertexBuffer {
    pub id: GLuint,
    deleted: std::cell::Cell<bool>,
    _thread: std::marker::PhantomData<Rc<()>>,
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
    /// # Safety
    /// Every byte of `s`, including padding, must be initialized.
    pub unsafe fn from_slice<T>(s: &[T]) -> Result<VertexBuffer, Error> {
        let vbo = unsafe { create_buffer(s, gl::ARRAY_BUFFER) }?;

        Ok(VertexBuffer {
            id: vbo,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        })
    }

    /// Delete the buffer when the context is current.
    /// `None` skips the GL call and reports failure; Drop logs the leak.
    pub fn destroy_if_current(&self, current: Option<Current>) -> bool {
        if self.deleted.get() {
            return false;
        }
        let Some(_current) = current else {
            log::debug!("orphan GL VBO {} skipped: no current context", self.id);
            self.deleted.set(true);
            return true;
        };
        let failed = super::raw::delete_buffers(1, &[self.id]).is_err();
        self.deleted.set(true);
        failed
    }
}

#[derive(Debug)]
pub struct IndexBuffer {
    id: GLuint,

    data_type: GLenum,
    deleted: std::cell::Cell<bool>,
    _thread: std::marker::PhantomData<Rc<()>>,
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
    /// # Safety
    /// Every byte, including padding, must be initialized and match `data_type`.
    pub unsafe fn from_slice<T>(s: &[T], data_type: GLenum) -> Result<IndexBuffer, Error> {
        let vbo = unsafe { create_buffer(s, gl::ELEMENT_ARRAY_BUFFER) }?;

        Ok(IndexBuffer {
            id: vbo,
            data_type,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        })
    }

    /// Delete the buffer when the context is current.
    /// `None` skips the GL call and reports failure; Drop logs the leak.
    pub fn destroy_if_current(&self, current: Option<Current>) -> bool {
        if self.deleted.get() {
            return false;
        }
        let Some(_current) = current else {
            log::debug!("orphan GL IBO {} skipped: no current context", self.id);
            self.deleted.set(true);
            return true;
        };
        let failed = super::raw::delete_buffers(1, &[self.id]).is_err();
        self.deleted.set(true);
        failed
    }
}

/// Caller guarantees that all source bytes, including padding, are initialized.
unsafe fn create_buffer<T>(source: &[T], target: GLenum) -> Result<GLuint, Error> {
    let mut id = 0;
    let generated = gen_buffers(1, std::slice::from_mut(&mut id));
    let cleanup = ScopeGuard::new(move || {
        if id != 0 && delete_buffers(1, &[id]).is_err() {
            log::debug!("failed to clean up incomplete GL buffer {id}");
        }
    });
    generated?;
    bind_buffer(target, id)?;
    let size = std::mem::size_of_val(source) as gl::types::GLsizeiptr;
    unsafe { buffer_data(target, size, source.as_ptr().cast(), gl::STATIC_DRAW) }?;
    super::check_frame()?;
    cleanup.dismiss();
    Ok(id)
}

#[derive(Debug, Copy, Clone)]
pub struct Attrib {
    pub id: GLuint,
}

impl super::context::GlObject for VertexArray {
    fn destroy_if_current(&mut self, current: Option<Current>) -> bool {
        VertexArray::destroy_if_current(self, current)
    }
}

impl super::context::GlObject for VertexBuffer {
    fn destroy_if_current(&mut self, current: Option<Current>) -> bool {
        VertexBuffer::destroy_if_current(self, current)
    }
}

impl super::context::GlObject for IndexBuffer {
    fn destroy_if_current(&mut self, current: Option<Current>) -> bool {
        IndexBuffer::destroy_if_current(self, current)
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::GlObject;
    use super::{Attrib, IndexBuffer, VertexBuffer};

    #[test]
    fn attrib_is_a_transparent_id() {
        let attr = Attrib { id: 7 };
        assert_eq!(attr.id, 7);
        let copy = attr;
        assert_eq!(copy.id, 7);
    }

    #[test]
    fn buffer_teardown_without_context_marks_leak() {
        let vbo = VertexBuffer {
            id: 11,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        };
        assert!(vbo.destroy_if_current(None));
        assert!(!vbo.destroy_if_current(None));
        let ibo = IndexBuffer {
            id: 13,
            data_type: gl::UNSIGNED_SHORT,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        };
        assert!(ibo.destroy_if_current(None));
        assert!(!ibo.destroy_if_current(None));
        let mut vbo = VertexBuffer {
            id: 17,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        };
        assert!(GlObject::destroy_if_current(&mut vbo, None));
        let mut ibo = IndexBuffer {
            id: 19,
            data_type: gl::UNSIGNED_SHORT,
            deleted: std::cell::Cell::new(false),
            _thread: std::marker::PhantomData,
        };
        assert!(GlObject::destroy_if_current(&mut ibo, None));
    }
}
