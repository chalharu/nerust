mod direct_rgb;
mod ntsc_simulator;
pub mod presentation;

pub use presentation::FilterTypeExt;

/// GPU texture width derived from `nerust_render_ntsc::SHADER_COLOR_COUNT` (= NES palette size).
pub const NTSC_TEXTURE_WIDTH: u32 = nerust_render_ntsc::SHADER_COLOR_COUNT as u32;
/// GPU texture height derived from `nerust_render_ntsc::SHADER_PHASE_COUNT * SHADER_PHASE_ENTRY_COUNT`.
pub const NTSC_TEXTURE_HEIGHT: u32 =
    (nerust_render_ntsc::SHADER_PHASE_COUNT * nerust_render_ntsc::SHADER_PHASE_ENTRY_COUNT) as u32;
