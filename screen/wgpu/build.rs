#[path = "src/shader_source.rs"]
mod shader_source;

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/shader.wgsl.in");
    println!("cargo:rerun-if-changed=src/shader_source.rs");

    let template =
        fs::read_to_string("src/shader.wgsl.in").expect("failed to read WGSL shader template");
    let shader = shader_source::render_shader_wgsl(&template);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    fs::write(out_dir.join("shader.wgsl"), shader).expect("failed to write generated WGSL shader");
    fs::write(
        out_dir.join("srgb_lut.bin"),
        shader_source::srgb_to_linear_lut_bytes(),
    )
    .expect("failed to write generated sRGB LUT");
}
