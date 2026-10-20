mod error;
pub mod raw;
pub mod vertex;

pub use error::Error as GlError;

/// Proof that the GL context is current on this thread. Destroy
/// entry points take `Option<Self>` so a detached teardown skips GL
/// calls instead of fabricating currency. Build only with
/// [`CurrentContext::assert_current`]: there is no other constructor,
/// so a token always traces back to a `ctx.is_current()` check at the
/// call site.
#[derive(Debug, Clone, Copy)]
pub struct CurrentContext(());

impl CurrentContext {
    /// Checked proof: `Some` only when the caller verified
    /// `ctx.is_current()`. `None` skips GL deletes and reports.
    /// Takes the `bool` verdict (not `&PossiblyCurrentContext`) so
    /// this crate stays free of a glutin dependency; call sites pass
    /// `ctx.is_current()` directly.
    pub fn assert_current(is_current: bool) -> Option<Self> {
        is_current.then_some(Self(()))
    }
}

use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    ptr,
};

use gl::types::{GLchar, GLenum, GLint, GLsizei, GLuint};

use self::{
    error::*,
    raw::{
        delete_program, get_active_attrib, get_active_uniform, get_attrib_location, get_programiv,
        get_uniform_location,
    },
};

/// Per-call drain in debug builds; release no-op to avoid a
/// pipeline stall on every GL call. Release builds rely on the
/// batched [`check_frame`] drain once per frame.
#[cfg(debug_assertions)]
fn gl_error_handle<T, F: FnMut() -> T>(mut func: F) -> Result<T, Error> {
    let result = func();
    gl_get_error()?;
    Ok(result)
}

/// Release fast path: run the call, skip the synchronous drain.
#[cfg(not(debug_assertions))]
fn gl_error_handle<T, F: FnMut() -> T>(mut func: F) -> Result<T, Error> {
    Ok(func())
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

/// Batched per-frame error drain. Call once at the end of a frame
/// instead of relying on the per-call drain in release builds.
/// In debug builds every wrapper still drains (see `gl_error_handle`);
/// in release wrappers are no-ops and this is the only check.
pub fn check_frame() -> Result<(), Error> {
    gl_get_error()
}

pub fn use_program(program: GLuint) -> Result<(), Error> {
    gl_error_handle(|| unsafe { gl::UseProgram(program) })
}

/// Which introspection query failed. Structural match target so
/// callers never parse the detail string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryStage {
    ActiveAttribs,
    ActiveAttrib,
    AttribName,
    AttribLocation,
    ActiveUniforms,
    ActiveUniform,
    UniformName,
    UniformLocation,
    LinkStatus,
    InfoLog,
}

/// Why a program failed to link. Typed so callers match on stage
/// instead of parsing a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderError {
    Compile { stage: &'static str, log: String },
    Link { log: String },
    Query { stage: QueryStage, detail: String },
}

impl core::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShaderError::Compile { stage, log } => write!(f, "{stage} compile failed: {log}"),
            ShaderError::Link { log } => write!(f, "link failed: {log}"),
            ShaderError::Query { stage, detail } => {
                write!(f, "query failed ({stage:?}): {detail}")
            }
        }
    }
}

impl std::error::Error for ShaderError {}

#[derive(Debug)]
pub struct Shader {
    program: GLuint,
    attributes: HashMap<String, GLuint>,
    uniforms: HashMap<String, GLint>,
    deleted: bool,
}

/// RAII shader-object guard: deleting happens on drop. A failed
/// fragment compile or link no longer orphans the vertex shader (or
/// both shaders on link failure); success paths detach then drop.
struct ShaderId(GLuint);

impl ShaderId {
    fn id(&self) -> GLuint {
        self.0
    }
}

impl Drop for ShaderId {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteShader(self.0);
        }
    }
}

/// Programs die with the view: rebuilds and detach must not orphan names.
/// Explicit `destroy_if_current` does the delete; Drop only reports leaks.
impl Shader {
    pub fn try_new(vert_src: &str, flag_src: &str) -> Result<Self, ShaderError> {
        let vtx = ShaderId(compile_shader(vert_src, gl::VERTEX_SHADER)?);
        let frag = ShaderId(compile_shader(flag_src, gl::FRAGMENT_SHADER)?);
        let program_id = link_program(vtx.id(), frag.id())?;
        // Link keeps its own copy: detach then delete via Drop.
        unsafe {
            gl::DetachShader(program_id, vtx.id());
            gl::DetachShader(program_id, frag.id());
        }
        drop(vtx);
        drop(frag);

        Ok(Self {
            program: program_id,
            attributes: get_attributes(program_id)?,
            uniforms: get_uniforms(program_id)?,
            deleted: false,
        })
    }

    /// Attribute location, if the linker kept the name. `None` instead
    /// of ambiguous `0`: location 0 is a valid attribute. Optimized-out
    /// names legitimately miss; callers skip or fail typed (no sentinel).
    pub fn attribute_opt(&self, key: &str) -> Option<GLuint> {
        self.attributes.get(key).copied()
    }

    /// Uniform location, if the linker kept the name. `None` instead
    /// of sentinel `-1`: callers skip the upload, which GL would ignore
    /// anyway, instead of threading a magic value.
    pub fn uniform_opt(&self, key: &str) -> Option<GLint> {
        self.uniforms.get(key).copied()
    }

    pub fn use_program(&self) -> Result<(), Error> {
        use_program(self.program)
    }

    pub fn id(&self) -> GLuint {
        self.program
    }

    /// Delete the program when the context is current.
    /// `None` skips the GL call and reports failure so the caller
    /// counts the orphan instead of deleting without a context.
    /// No GL call without a current context; Drop reports the leak.
    pub fn destroy_if_current(&mut self, current: Option<CurrentContext>) -> bool {
        if self.deleted {
            return false;
        }
        let Some(_current) = current else {
            log::debug!(
                "orphan GL program {} skipped: no current context",
                self.program
            );
            self.deleted = true;
            return true;
        };
        let failed = delete_program(self.program).is_err();
        self.deleted = true;
        failed
    }
}

/// No GL calls here: without a current context delete would fail.
impl Drop for Shader {
    fn drop(&mut self) {
        if !self.deleted {
            log::debug!("orphan GL program {} leaked at drop", self.program);
        }
    }
}

fn get_attributes(program_id: GLuint) -> Result<HashMap<String, GLuint>, ShaderError> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_ATTRIBUTES, &mut count).map_err(|e| {
        ShaderError::Query {
            stage: QueryStage::ActiveAttribs,
            detail: e.to_string(),
        }
    })?;

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
        .map_err(|e| ShaderError::Query {
            stage: QueryStage::ActiveAttrib,
            detail: e.to_string(),
        })?;
        let name = String::from(
            unsafe { CStr::from_ptr(name_buf.as_ptr()) }
                .to_str()
                .map_err(|e| ShaderError::Query {
                    stage: QueryStage::AttribName,
                    detail: e.to_string(),
                })?,
        );
        let location =
            get_attrib_location(program_id, unsafe { CStr::from_ptr(name_buf.as_ptr()) }).map_err(
                |e| ShaderError::Query {
                    stage: QueryStage::AttribLocation,
                    detail: e.to_string(),
                },
            )?;
        let location = u32::try_from(location).map_err(|e| ShaderError::Query {
            stage: QueryStage::AttribLocation,
            detail: format!("attribute {name} has no location: {e}"),
        })?;
        let _ = result.insert(name, location);
    }
    Ok(result)
}

fn get_uniforms(program_id: GLuint) -> Result<HashMap<String, GLint>, ShaderError> {
    let mut count: GLint = 0;
    get_programiv(program_id, gl::ACTIVE_UNIFORMS, &mut count).map_err(|e| ShaderError::Query {
        stage: QueryStage::ActiveUniforms,
        detail: e.to_string(),
    })?;

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
        .map_err(|e| ShaderError::Query {
            stage: QueryStage::ActiveUniform,
            detail: e.to_string(),
        })?;
        let name = String::from(
            unsafe { CStr::from_ptr(name_buf.as_ptr()) }
                .to_str()
                .map_err(|e| ShaderError::Query {
                    stage: QueryStage::UniformName,
                    detail: e.to_string(),
                })?,
        );
        let location =
            get_uniform_location(program_id, unsafe { CStr::from_ptr(name_buf.as_ptr()) })
                .map_err(|e| ShaderError::Query {
                    stage: QueryStage::UniformLocation,
                    detail: e.to_string(),
                })?;
        let _ = result.insert(name, location);
    }

    Ok(result)
}

fn info_log_buffer(len: GLint) -> Vec<u8> {
    vec![0; len.max(0) as usize]
}

fn trim_info_log(buf: &mut Vec<u8>, written: GLsizei) -> String {
    let written = (written.max(0) as usize).min(buf.len());
    buf.truncate(written);
    if buf.last() == Some(&0) {
        let _ = buf.pop();
    }
    String::from_utf8_lossy(buf.as_slice()).into_owned()
}

fn compile_shader(src: &str, ty: GLenum) -> Result<GLuint, ShaderError> {
    let shader = unsafe { gl::CreateShader(ty) };
    let c_str = CString::new(src.as_bytes()).map_err(|e| ShaderError::Compile {
        stage: if ty == gl::VERTEX_SHADER {
            "vertex"
        } else {
            "fragment"
        },
        log: e.to_string(),
    })?;
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
            gl::GetShaderInfoLog(shader, len, &mut written, buf.as_mut_ptr().cast());
            gl::DeleteShader(shader);
            return Err(ShaderError::Compile {
                stage: if ty == gl::VERTEX_SHADER {
                    "vertex"
                } else {
                    "fragment"
                },
                log: trim_info_log(&mut buf, written),
            });
        }
    }
    Ok(shader)
}

fn link_program(vs: GLuint, fs: GLuint) -> Result<GLuint, ShaderError> {
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vs);
        gl::AttachShader(program, fs);
        gl::LinkProgram(program);

        let mut status = GLint::from(gl::FALSE);
        get_programiv(program, gl::LINK_STATUS, &mut status).map_err(|e| ShaderError::Query {
            stage: QueryStage::LinkStatus,
            detail: e.to_string(),
        })?;

        if status != GLint::from(gl::TRUE) {
            let mut len: GLint = 0;
            get_programiv(program, gl::INFO_LOG_LENGTH, &mut len).map_err(|e| {
                ShaderError::Query {
                    stage: QueryStage::InfoLog,
                    detail: e.to_string(),
                }
            })?;
            let mut written = 0;
            let mut buf = info_log_buffer(len);
            gl::GetProgramInfoLog(program, len, &mut written, buf.as_mut_ptr().cast());
            gl::DeleteProgram(program);
            return Err(ShaderError::Link {
                log: trim_info_log(&mut buf, written),
            });
        }
        Ok(program)
    }
}
