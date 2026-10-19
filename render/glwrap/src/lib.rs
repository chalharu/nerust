mod error;
pub mod raw;
pub mod vertex;

pub use error::Error as GlError;

/// Proof that the GL context is current on this thread. Destroy
/// entry points take this instead of a bare `bool` so the provenance
/// (`ctx.is_current()` at the call site) cannot be erased or
/// transposed at a later call.
#[derive(Debug, Clone, Copy)]
pub struct CurrentContext(());

impl CurrentContext {
    /// Assert currentness once, then thread the token to every
    /// destroy call in the teardown sequence.
    pub fn current() -> Self {
        Self(())
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

/// Programs die with the view: rebuilds and detach must not orphan names.
/// Explicit `destroy_if_current` does the delete; Drop only reports leaks.
impl Shader {
    pub fn try_new(vert_src: &str, flag_src: &str) -> Result<Self, ShaderError> {
        let vtx_shader_id = compile_shader(vert_src, gl::VERTEX_SHADER)?;
        let flag_shader_id = compile_shader(flag_src, gl::FRAGMENT_SHADER)?;
        let program_id = link_program(vtx_shader_id, flag_shader_id)?;

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

    /// Delete the program when the context is current.
    /// No GL call without a current context; Drop reports the leak.
    pub fn destroy_if_current(&mut self, _current: CurrentContext) -> bool {
        if self.deleted {
            return false;
        }
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
