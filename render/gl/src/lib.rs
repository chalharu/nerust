mod error;
mod frame_textures;
pub(crate) mod frame_view;
mod layout;
mod mat4;
mod osd_overlay;
pub(crate) mod osd_view;
mod shader_compiler;
pub(crate) mod surface_view;
mod vec2d;
mod vertex_data;
pub(crate) mod view;

#[cfg(all(test, unix))]
mod test_display;

mod renderer;

pub use renderer::{GlFactory, GlRenderer};
