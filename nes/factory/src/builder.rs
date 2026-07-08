use std::collections::HashMap;

use nerust_core_traits::audio::AudioBackend;
use nerust_core_traits::factory::settings::FactorySettingsView;
use nerust_core_traits::factory::{CoreParts, FactoryError};
use nerust_input_traits::{EmuInput, GuiInput};
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_render_base::{FilterType, LogicalSize, VideoRenderProfile};

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    gui_input: GuiInput,
    emu_input: EmuInput,
    field_map: HashMap<(&'static str, &'static str), usize>,
) -> Result<CoreParts, FactoryError> {
    let filter = crate::settings::filter_type_from_bytes(&view.system_config_bytes);

    let device = NesPadDevice::new();
    let (render_profile, palette) = compute_render_profile(filter);
    let mut speaker = speaker;
    speaker.start();
    let core = NesConsoleCore::new_empty(Box::new(device), speaker, emu_input);
    Ok(CoreParts {
        core: Box::new(core),
        gui_input,
        field_map,
        render_profile,
        palette,
    })
}

fn compute_render_profile(filter_type: FilterType) -> (VideoRenderProfile, Box<[u32; 256]>) {
    let source_logical_size = LogicalSize {
        width: 256,
        height: 240,
    };
    let layout = filter_type.layout(source_logical_size);
    let assets = filter_type.palette_console_video_assets();
    let ntsc_packed_rgba8 = assets
        .packed_ntsc_rgba8()
        .map(|data| data.to_vec().into_boxed_slice());
    let render_profile = VideoRenderProfile {
        source_logical_size: layout.source_logical_size,
        logical_size: layout.logical_size,
        physical_size: layout.physical_size,
        frame_format: nerust_render_base::VideoFrameFormat::Palette,
        ntsc_packed_rgba8,
    };
    let mut palette = [0u32; 256];
    let rgba8 = assets.palette_rgba8();
    for (i, entry) in palette.iter_mut().enumerate().take(64) {
        let pos = i * 4;
        *entry = u32::from(rgba8[pos]) << 24
            | u32::from(rgba8[pos + 1]) << 16
            | u32::from(rgba8[pos + 2]) << 8
            | u32::from(rgba8[pos + 3]);
    }
    (render_profile, Box::new(palette))
}
