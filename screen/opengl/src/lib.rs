mod mat4;
mod vec2d;
mod vertex_data;

use self::mat4::Mat4;
use self::vec2d::Vec2D;
use self::vertex_data::VertexData;
use gl::types::GLint;
use nerust_console::video::VideoRenderProfile;
use nerust_glwrap::Shader;
use nerust_glwrap::raw::*;
use nerust_glwrap::vertex::*;
use std::ffi::CStr;
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;

const DIRECT_FRAGMENT_DESKTOP: &str = r#"
uniform sampler2D frame_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void) {
    frag_color = texture(frame_texture, vuv);
}
"#;

const DIRECT_FRAGMENT_COMPAT: &str = r#"
uniform sampler2D frame_texture;
varying NERUST_MEDIUMP vec2 vuv;

void main(void) {
    gl_FragColor = texture2D(frame_texture, vuv);
}
"#;

fn allocate(size: usize) -> Box<[u8]> {
    vec![0; size].into_boxed_slice()
}

#[derive(Debug)]
pub struct GlView {
    frame_texture: u32,
    shader: Option<Shader>,
    use_vao: bool,
    vba: Option<VertexArray>,
    vbo: Option<Rc<VertexBuffer>>,
    logical_width: i32,
    logical_height: i32,
}

impl GlView {
    pub fn new() -> Self {
        Self {
            frame_texture: 0,
            shader: None,
            use_vao: false,
            vba: None,
            vbo: None,
            logical_width: 0,
            logical_height: 0,
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.use_vao = value;
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), String> {
        let logical_size = render_profile.logical_size;
        let shader = compile_shader_program();
        self.logical_width = logical_size.width as i32;
        self.logical_height = logical_size.height as i32;
        shader.use_program();
        clear_color(0.0, 0.0, 0.0, 1.0).unwrap();

        let mut texture_names = [0; 1];
        gen_textures(1, texture_names.as_mut_ptr()).unwrap();
        self.frame_texture = texture_names[0];
        pixel_storei(gl::UNPACK_ALIGNMENT, 1).unwrap();
        configure_rgba_texture(
            0,
            self.frame_texture,
            logical_size.width,
            logical_size.height,
            allocate(logical_size.width * logical_size.height * 4).as_ref(),
        );

        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 1.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(1.0, 0.0)),
            VertexData::new(Vec2D::new(1.0, -1.0), Vec2D::new(1.0, 1.0)),
        ];

        let vbo = Rc::new(VertexBuffer::from_slice(&vertex_data).unwrap());
        if self.use_vao {
            let vbo = vbo.clone();
            self.vba = Some(
                VertexArray::new(|vaic| {
                    vaic.bind_vbo(vbo, |vac| {
                        vac.attr_pointer(
                            Attrib {
                                id: shader.get_attribute("position"),
                            },
                            2,
                            gl::FLOAT,
                            16,
                            0,
                        )?;
                        vac.attr_pointer(
                            Attrib {
                                id: shader.get_attribute("uv"),
                            },
                            2,
                            gl::FLOAT,
                            16,
                            8,
                        )
                    })
                })
                .unwrap(),
            );
        } else {
            vertex_attrib_pointer(
                shader.get_attribute("position"),
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                ptr::null(),
            )
            .unwrap();
            enable_vertex_attrib_array(shader.get_attribute("position")).unwrap();

            vertex_attrib_pointer(
                shader.get_attribute("uv"),
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                8 as *const c_void,
            )
            .unwrap();
            enable_vertex_attrib_array(shader.get_attribute("uv")).unwrap();
            self.vbo = Some(vbo.clone());
        }

        uniform_matrix_4fv(
            shader.get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        )
        .unwrap();
        uniform_1i(shader.get_uniform("frame_texture"), 0).unwrap();
        self.shader = Some(shader);
        Ok(())
    }

    pub fn on_update(&self, screen_ptr: *const u8) {
        self.shader.as_ref().unwrap().use_program();
        active_texture(gl::TEXTURE0).unwrap();
        bind_texture(gl::TEXTURE_2D, self.frame_texture).unwrap();
        if self.use_vao {
            self.vba.as_ref().unwrap().bind_vao(|_vac| Ok(())).unwrap();
        } else {
            bind_buffer(gl::ARRAY_BUFFER, self.vbo.as_ref().unwrap().id).unwrap();
        }
        clear(gl::COLOR_BUFFER_BIT).unwrap();
        tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            self.logical_width,
            self.logical_height,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            screen_ptr as *const c_void,
        )
        .unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
    }

    pub fn on_resize(
        &mut self,
        scale_x: f32,
        scale_y: f32,
        viewport_width: i32,
        viewport_height: i32,
    ) {
        self.shader.as_ref().unwrap().use_program();
        if self.use_vao {
            self.vba.as_ref().unwrap().bind_vao(|_vac| Ok(())).unwrap();
        } else {
            bind_buffer(gl::ARRAY_BUFFER, self.vbo.as_ref().unwrap().id).unwrap();
        }
        viewport(0, 0, viewport_width, viewport_height).unwrap();
        uniform_matrix_4fv(
            self.shader.as_ref().unwrap().get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        )
        .unwrap();
    }

    pub fn on_close(&mut self) {
        if self.frame_texture != 0 {
            delete_textures(1, &self.frame_texture).unwrap();
        }
    }
}

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}

fn configure_rgba_texture(unit: u32, texture: u32, width: usize, height: usize, data: &[u8]) {
    active_texture(gl::TEXTURE0 + unit).unwrap();
    bind_texture(gl::TEXTURE_2D, texture).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();
    tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA as GLint,
        width as i32,
        height as i32,
        0,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        data.as_ptr() as *const _,
    )
    .unwrap();
}

fn build_glsl_source(
    source: &str,
    version_override: Option<&str>,
    extra_preamble: &[&str],
) -> String {
    let (version_line, body) = source
        .split_once('\n')
        .expect("GLSL source must start with a version line");
    compose_glsl_source(
        version_override.unwrap_or(version_line),
        extra_preamble,
        &[body],
    )
}

fn compose_glsl_source(version_line: &str, extra_preamble: &[&str], parts: &[&str]) -> String {
    let mut output = String::new();
    output.push_str(version_line);
    output.push('\n');
    for line in extra_preamble {
        output.push_str(line);
        output.push('\n');
    }
    for part in parts {
        output.push_str(part);
        if !part.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

fn compile_shader_program() -> Shader {
    let context_version = gl_string(gl::VERSION);
    let shading_version = gl_string(gl::SHADING_LANGUAGE_VERSION);
    let is_gles = is_gles_context(context_version.as_deref());

    log::info!(
        "initializing OpenGL renderer with context {:?} and shading language {:?}",
        context_version,
        shading_version
    );

    let candidates: Vec<(&str, String, String)> = if is_gles {
        vec![
            (
                "gles3",
                build_glsl_source(
                    include_str!("vertex_desktop.glsl"),
                    Some("#version 300 es"),
                    &["precision mediump float;"],
                ),
                compose_glsl_source(
                    "#version 300 es",
                    &["precision mediump float;"],
                    &[DIRECT_FRAGMENT_DESKTOP],
                ),
            ),
            (
                "gles2",
                include_str!("vertex.glsl").to_owned(),
                compose_glsl_source(
                    "#version 100",
                    &["#define NERUST_MEDIUMP mediump"],
                    &[DIRECT_FRAGMENT_COMPAT],
                ),
            ),
        ]
    } else {
        vec![
            (
                "desktop-core",
                include_str!("vertex_desktop.glsl").to_owned(),
                compose_glsl_source("#version 150", &[], &[DIRECT_FRAGMENT_DESKTOP]),
            ),
            (
                "desktop-legacy",
                include_str!("vertex_legacy.glsl").to_owned(),
                compose_glsl_source(
                    "#version 120",
                    &["#define NERUST_MEDIUMP"],
                    &[DIRECT_FRAGMENT_COMPAT],
                ),
            ),
        ]
    };

    let mut errors = Vec::new();
    for (name, vertex, fragment) in candidates {
        match Shader::try_new(vertex.as_str(), fragment.as_str()) {
            Ok(shader) => {
                log::info!("selected {name} shader pipeline");
                return shader;
            }
            Err(err) => errors.push(format!("{name}: {err}")),
        }
    }

    panic!(
        "failed to compile shader pipeline for context {:?} / {:?}: {}",
        context_version,
        shading_version,
        errors.join(" | ")
    );
}

fn is_gles_context(version: Option<&str>) -> bool {
    version.is_some_and(|value| value.contains("OpenGL ES"))
}

fn gl_string(name: u32) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::{build_glsl_source, compose_glsl_source, is_gles_context};

    #[test]
    fn detects_gles_context_strings() {
        assert!(is_gles_context(Some("OpenGL ES 3.2 Mesa 24.1.0")));
        assert!(!is_gles_context(Some("4.6 (Core Profile) Mesa 24.1.0")));
        assert!(!is_gles_context(None));
    }

    #[test]
    fn inserts_extra_preamble_after_version_line() {
        let source = build_glsl_source(
            "#version 120\nvoid main(void) {}\n",
            None,
            &["#define TEST 1"],
        );
        assert_eq!(source, "#version 120\n#define TEST 1\nvoid main(void) {}\n");
    }

    #[test]
    fn composes_glsl_parts_in_order() {
        let source = compose_glsl_source(
            "#version 120",
            &["#define NERUST_MEDIUMP"],
            &["void helper(void) {}\n", "void main(void) {}\n"],
        );

        assert_eq!(
            source,
            "#version 120\n#define NERUST_MEDIUMP\nvoid helper(void) {}\nvoid main(void) {}\n"
        );
    }
}
