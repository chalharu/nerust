mod direct_rgb;
mod ntsc_simulator;
pub mod presentation;

pub use presentation::FilterTypeExt;

pub const NTSC_TEXTURE_WIDTH: u32 = nerust_render_ntsc::SHADER_COLOR_COUNT as u32;
pub const NTSC_TEXTURE_HEIGHT: u32 =
    (nerust_render_ntsc::SHADER_PHASE_COUNT * nerust_render_ntsc::SHADER_PHASE_ENTRY_COUNT) as u32;
