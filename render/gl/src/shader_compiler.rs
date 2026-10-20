//! Game-pipeline shader compilation: pure candidate assembly plus the link step.
//! [`super::view::GlView`] coordinates uploads through the returned program.

use nerust_glwrap::Shader;

pub(crate) const DIRECT_FRAGMENT_DESKTOP: &str = r#"
uniform sampler2D frame_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void) {
    frag_color = texture(frame_texture, vuv);
}
"#;

pub(crate) const DIRECT_FRAGMENT_COMPAT: &str = r#"
uniform sampler2D frame_texture;
varying NERUST_MEDIUMP vec2 vuv;

void main(void) {
    gl_FragColor = texture2D(frame_texture, vuv);
}
"#;

pub(crate) const PALETTE_FRAGMENT_DESKTOP: &str = include_str!("fragment_desktop_combined.glsl");

pub(crate) const OSD_FRAGMENT_DESKTOP: &str = r#"
uniform sampler2D osd_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void) {
    frag_color = texture(osd_texture, vuv);
}
"#;

pub(crate) const OSD_FRAGMENT_COMPAT: &str = r#"
uniform sampler2D osd_texture;
varying NERUST_MEDIUMP vec2 vuv;

void main(void) {
    gl_FragColor = texture2D(osd_texture, vuv);
}
"#;

pub(crate) fn build_glsl_source(
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

pub(crate) fn compose_glsl_source(
    version_line: &str,
    extra_preamble: &[&str],
    parts: &[&str],
) -> String {
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

pub(crate) fn compile_shader_program(
    is_palette: bool,
    is_gles: bool,
) -> Result<Shader, crate::error::ViewError> {
    use crate::error::ViewError;
    use nerust_glwrap::raw::gl_string;

    let context_version = gl_string(gl::VERSION);
    let shading_version = gl_string(gl::SHADING_LANGUAGE_VERSION);

    log::info!(
        "initializing OpenGL renderer with context {:?} and shading language {:?} (palette={is_palette})",
        context_version,
        shading_version,
    );

    let fragment_desktop = if is_palette {
        PALETTE_FRAGMENT_DESKTOP
    } else {
        DIRECT_FRAGMENT_DESKTOP
    };
    let fragment_compat = DIRECT_FRAGMENT_COMPAT;

    let candidates: Vec<(&str, String, String)> = if is_gles {
        let mut gles = vec![(
            "gles3",
            build_glsl_source(
                include_str!("vertex_desktop.glsl"),
                Some("#version 300 es"),
                &["precision mediump float;"],
            ),
            compose_glsl_source(
                "#version 300 es",
                &["precision mediump float;"],
                &[fragment_desktop],
            ),
        )];
        if !is_palette {
            gles.push((
                "gles2",
                include_str!("vertex.glsl").to_owned(),
                compose_glsl_source(
                    "#version 100",
                    &["#define NERUST_MEDIUMP mediump"],
                    &[fragment_compat],
                ),
            ));
        }
        gles
    } else {
        let mut desktop = vec![(
            "desktop-core",
            include_str!("vertex_desktop.glsl").to_owned(),
            compose_glsl_source("#version 150", &[], &[fragment_desktop]),
        )];
        if !is_palette {
            desktop.push((
                "desktop-legacy",
                include_str!("vertex_legacy.glsl").to_owned(),
                compose_glsl_source(
                    "#version 120",
                    &["#define NERUST_MEDIUMP"],
                    &[fragment_compat],
                ),
            ));
        }
        desktop
    };

    let mut errors = Vec::new();
    for (name, vertex, fragment) in candidates {
        match Shader::try_new(vertex.as_str(), fragment.as_str()) {
            Ok(shader) => {
                log::info!("selected {name} shader pipeline");
                return Ok(shader);
            }
            Err(err) => {
                log::warn!("{name} shader pipeline unavailable: {err}");
                errors.push(format!("{name}: {err}"));
            }
        }
    }

    log::warn!("no GL shader pipeline compiled: {}", errors.join("; "));
    Err(ViewError::NoProgram { program: "game" })
}

pub(crate) fn compile_osd_program(is_gles: bool) -> Option<Shader> {
    let candidates: Vec<(&str, String, String)> = if is_gles {
        vec![
            (
                "osd-gles3",
                build_glsl_source(
                    include_str!("vertex_desktop.glsl"),
                    Some("#version 300 es"),
                    &["precision mediump float;"],
                ),
                compose_glsl_source(
                    "#version 300 es",
                    &["precision mediump float;"],
                    &[OSD_FRAGMENT_DESKTOP],
                ),
            ),
            (
                "osd-gles2",
                include_str!("vertex.glsl").to_owned(),
                compose_glsl_source(
                    "#version 100",
                    &["#define NERUST_MEDIUMP mediump"],
                    &[OSD_FRAGMENT_COMPAT],
                ),
            ),
        ]
    } else {
        vec![(
            "osd-desktop",
            include_str!("vertex_desktop.glsl").to_owned(),
            compose_glsl_source("#version 150", &[], &[OSD_FRAGMENT_DESKTOP]),
        )]
    };
    for (name, vertex, fragment) in candidates {
        match Shader::try_new(vertex.as_str(), fragment.as_str()) {
            Ok(shader) => {
                log::info!("selected {name} osd pipeline");
                return Some(shader);
            }
            Err(err) => log::warn!("osd shader {name} unavailable: {err}"),
        }
    }
    None
}

pub(crate) fn is_gles_context(version: Option<&str>) -> bool {
    version.is_some_and(|value| value.contains("OpenGL ES"))
}

/// UNPACK_ROW_LENGTH is core on desktop GL and arrived in GLES3;
/// older ES has no row stride control, so frames repack on CPU.
pub(crate) fn supports_unpack_row_length(version: Option<&str>) -> bool {
    let Some(value) = version else {
        return false;
    };
    if !value.contains("OpenGL ES") {
        return true;
    }
    value.contains("OpenGL ES 3")
}
