//! Context-local entry points. Constants/types come from `gl`; its global loader is never used.
#![allow(non_snake_case)]
use ::gl::types::{
    GLbitfield, GLboolean, GLchar, GLenum, GLfloat, GLint, GLsizei, GLsizeiptr, GLubyte, GLuint,
};
/// Re-exported GL surface: the `types` module plus every constant named
/// downstream. Explicit so unused GL surface never leaks; additions fail loudly.
pub use ::gl::{
    ACTIVE_ATTRIBUTE_MAX_LENGTH, ACTIVE_ATTRIBUTES, ACTIVE_TEXTURE, ACTIVE_UNIFORM_MAX_LENGTH,
    ACTIVE_UNIFORMS, ARRAY_BUFFER, ARRAY_BUFFER_BINDING, BLEND, BLEND_DST_ALPHA, BLEND_DST_RGB,
    BLEND_SRC_ALPHA, BLEND_SRC_RGB, CLAMP_TO_EDGE, COLOR_BUFFER_BIT, COMPILE_STATUS,
    COMPUTE_WORK_GROUP_SIZE, CONTEXT_LOST, CURRENT_PROGRAM, ELEMENT_ARRAY_BUFFER, FALSE, FLOAT,
    FRAGMENT_SHADER, INFO_LOG_LENGTH, INVALID_ENUM, INVALID_FRAMEBUFFER_OPERATION,
    INVALID_OPERATION, INVALID_VALUE, LINK_STATUS, MAX_TEXTURE_SIZE, MAX_VIEWPORT_DIMS, NEAREST,
    NO_ERROR, ONE, ONE_MINUS_SRC_ALPHA, OUT_OF_MEMORY, PACK_ALIGNMENT, PACK_ROW_LENGTH,
    PACK_SKIP_PIXELS, PACK_SKIP_ROWS, PIXEL_PACK_BUFFER, PIXEL_UNPACK_BUFFER, R8, RED, RGBA,
    SHADING_LANGUAGE_VERSION, SRC_ALPHA, STACK_OVERFLOW, STACK_UNDERFLOW, STATIC_DRAW, TEXTURE_2D,
    TEXTURE_MAG_FILTER, TEXTURE_MIN_FILTER, TEXTURE_WRAP_S, TEXTURE_WRAP_T, TEXTURE0,
    TRIANGLE_STRIP, TRUE, UNPACK_ALIGNMENT, UNPACK_ROW_LENGTH, UNPACK_SKIP_PIXELS,
    UNPACK_SKIP_ROWS, UNSIGNED_BYTE, UNSIGNED_SHORT, VERSION, VERTEX_ARRAY_BINDING, VERTEX_SHADER,
    VIEWPORT, types,
};
use std::{cell::RefCell, ffi::c_void, rc::Rc};

thread_local! {
    static CURRENT: RefCell<Option<Rc<Table>>> = const { RefCell::new(None) };
}

macro_rules! entry_points {
    ($($(#[$attr:meta])* $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)+) => {
        struct Table { $($name: Option<unsafe extern "system" fn($($ty),*) -> $ret>,)+ }
        impl Table {
            unsafe fn load(mut loader: impl FnMut(&'static str) -> *const c_void) -> Self {
                Self { $($name: {
                    let address = loader(concat!("gl", stringify!($name)));
                    if address.is_null() { None } else {
                        Some(unsafe { std::mem::transmute::<*const c_void, unsafe extern "system" fn($($ty),*) -> $ret>(address) })
                    }
                },)+ }
            }
        }
        $(
            /// # Safety
            /// The native context must be current and all OpenGL argument/memory requirements apply.
            $(#[$attr])*
            pub unsafe fn $name($($arg: $ty),*) -> $ret {
                let table = CURRENT.with(|slot| slot.borrow().clone()).expect("no current GL dispatch");
                let function = table.$name.expect(concat!("missing GL entry point: ", stringify!($name)));
                unsafe { function($($arg),*) }
            }
        )+
    }
}

entry_points! {
    GetError() -> GLenum;
    GetString(name: GLenum) -> *const GLubyte;
    GetIntegerv(name: GLenum, value: *mut GLint) -> ();
    GetProgramiv(program: GLuint, name: GLenum, value: *mut GLint) -> ();
    GetActiveAttrib(program: GLuint, index: GLuint, capacity: GLsizei, length: *mut GLsizei, size: *mut GLint, kind: *mut GLenum, name: *mut GLchar) -> ();
    GetActiveUniform(program: GLuint, index: GLuint, capacity: GLsizei, length: *mut GLsizei, size: *mut GLint, kind: *mut GLenum, name: *mut GLchar) -> ();
    GetAttribLocation(program: GLuint, name: *const GLchar) -> GLint;
    GetUniformLocation(program: GLuint, name: *const GLchar) -> GLint;
    GenTextures(count: GLsizei, names: *mut GLuint) -> ();
    DeleteTextures(count: GLsizei, names: *const GLuint) -> ();
    BindTexture(target: GLenum, name: GLuint) -> ();
    ActiveTexture(unit: GLenum) -> ();
    PixelStorei(name: GLenum, value: GLint) -> ();
    #[allow(clippy::too_many_arguments, reason = "Matches the OpenGL entry point signature")]
    TexImage2D(target: GLenum, level: GLint, internal: GLint, width: GLsizei, height: GLsizei, border: GLint, format: GLenum, kind: GLenum, pixels: *const c_void) -> ();
    #[allow(clippy::too_many_arguments, reason = "Matches the OpenGL entry point signature")]
    TexSubImage2D(target: GLenum, level: GLint, x: GLint, y: GLint, width: GLsizei, height: GLsizei, format: GLenum, kind: GLenum, pixels: *const c_void) -> ();
    TexParameteri(target: GLenum, name: GLenum, value: GLint) -> ();
    GetTexImage(target: GLenum, level: GLint, format: GLenum, kind: GLenum, pixels: *mut c_void) -> ();
    GenBuffers(count: GLsizei, names: *mut GLuint) -> ();
    DeleteBuffers(count: GLsizei, names: *const GLuint) -> ();
    BindBuffer(target: GLenum, name: GLuint) -> ();
    BufferData(target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum) -> ();
    GenVertexArrays(count: GLsizei, names: *mut GLuint) -> ();
    DeleteVertexArrays(count: GLsizei, names: *const GLuint) -> ();
    BindVertexArray(name: GLuint) -> ();
    EnableVertexAttribArray(index: GLuint) -> ();
    VertexAttribPointer(index: GLuint, size: GLint, kind: GLenum, normalized: GLboolean, stride: GLsizei, pointer: *const c_void) -> ();
    Uniform1i(location: GLint, value: GLint) -> ();
    Uniform2f(location: GLint, x: GLfloat, y: GLfloat) -> ();
    UniformMatrix4fv(location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) -> ();
    ClearColor(r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat) -> ();
    Clear(mask: GLbitfield) -> ();
    DrawArrays(mode: GLenum, first: GLint, count: GLsizei) -> ();
    DrawElements(mode: GLenum, count: GLsizei, kind: GLenum, indices: *const c_void) -> ();
    Viewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) -> ();
    Enable(cap: GLenum) -> ();
    Disable(cap: GLenum) -> ();
    IsEnabled(cap: GLenum) -> GLboolean;
    BlendFunc(source: GLenum, destination: GLenum) -> ();
    BlendFuncSeparate(source_rgb: GLenum, destination_rgb: GLenum, source_alpha: GLenum, destination_alpha: GLenum) -> ();
    UseProgram(program: GLuint) -> ();
    CreateShader(kind: GLenum) -> GLuint;
    DeleteShader(shader: GLuint) -> ();
    ShaderSource(shader: GLuint, count: GLsizei, strings: *const *const GLchar, lengths: *const GLint) -> ();
    CompileShader(shader: GLuint) -> ();
    GetShaderiv(shader: GLuint, name: GLenum, value: *mut GLint) -> ();
    GetShaderInfoLog(shader: GLuint, capacity: GLsizei, length: *mut GLsizei, text: *mut GLchar) -> ();
    CreateProgram() -> GLuint;
    DeleteProgram(program: GLuint) -> ();
    AttachShader(program: GLuint, shader: GLuint) -> ();
    BindAttribLocation(program: GLuint, index: GLuint, name: *const GLchar) -> ();
    DetachShader(program: GLuint, shader: GLuint) -> ();
    LinkProgram(program: GLuint) -> ();
    GetProgramInfoLog(program: GLuint, capacity: GLsizei, length: *mut GLsizei, text: *mut GLchar) -> ();
    ReadPixels(x: GLint, y: GLint, width: GLsizei, height: GLsizei, format: GLenum, kind: GLenum, pixels: *mut c_void) -> ();
    IsBuffer(buffer: GLuint) -> GLboolean;
    IsVertexArray(array: GLuint) -> GLboolean;
    IsProgram(program: GLuint) -> GLboolean;
}

/// A function table for one native context. `Rc` keeps it thread-affine.
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<nerust_glwrap::Context>();
/// ```
#[derive(Clone)]
pub struct Context(Rc<Table>);

impl Context {
    /// # Safety
    /// The loader must supply ABI-correct functions for one native GL context.
    pub unsafe fn load_with(loader: impl FnMut(&'static str) -> *const c_void) -> Self {
        Self(Rc::new(unsafe { Table::load(loader) }))
    }

    /// # Safety
    /// This table's native context must remain current until the guard drops.
    pub unsafe fn enter(&self) -> DispatchGuard {
        DispatchGuard(CURRENT.with(|slot| slot.replace(Some(self.0.clone()))))
    }

    /// # Safety
    /// This table's native context must be current for the entire callback.
    pub unsafe fn with_current<T>(&self, callback: impl FnOnce() -> T) -> T {
        let _guard = unsafe { self.enter() };
        callback()
    }
}

pub struct DispatchGuard(Option<Rc<Table>>);

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        CURRENT.with(|slot| {
            let _ = slot.replace(self.0.take());
        });
    }
}

pub(crate) fn has_current() -> bool {
    CURRENT.with(|slot| slot.borrow().is_some())
}
