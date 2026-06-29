use std::sync::{Arc, Mutex, atomic::AtomicBool};

use nerust_core_traits::audio::AudioBackend;
use nerust_core_traits::factory::FactoryError;
use nerust_emu_thread::EmuThread;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::emu_core::EmuCore;
use nerust_input_traits::SystemInputAdapter;
use nerust_nes_controller::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_render_base::{FilterType, FrameBuffer, LogicalSize, PixelFormat, VideoRenderProfile};

use crate::adapter::NesAdapter;

pub(crate) fn create_core_and_adapter(
    settings: &SettingsSnapshot,
    speaker: Box<dyn AudioBackend>,
) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError> {
    let filter = crate::settings::filter_type(&settings.shared);
    let cell = Arc::new(NesInputCell::new());
    let device = NesPadDevice::new(SharedNesInputCell(cell.clone()));

    let core = build_emu_core(speaker, filter, Box::new(device))?;
    let adapter = Box::new(NesAdapter::new(cell));
    Ok((core, adapter))
}

fn build_emu_core(
    speaker: Box<dyn nerust_core_traits::audio::AudioBackend + Send>,
    filter_type: FilterType,
    controller: Box<dyn nerust_nes_core::controller::Controller + Send>,
) -> Result<EmuCore, FactoryError> {
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
    let pixel_format = PixelFormat::PaletteIndex {
        palette: Box::new(palette),
    };
    let src_w = source_logical_size.width;
    let src_h = source_logical_size.height;

    let shared_fb = Arc::new(Mutex::new(FrameBuffer::with_capacity(
        src_w,
        src_h,
        pixel_format.clone(),
    )));
    if let Ok(mut guard) = shared_fb.lock() {
        guard.resize(src_w, src_h);
        guard.resize_data(src_w * src_h);
    }

    let mut disp_fb = FrameBuffer::with_capacity(src_w, src_h, pixel_format.clone());
    disp_fb.resize(src_w, src_h);
    disp_fb.resize_data(src_w * src_h);

    let mut speaker = speaker;
    speaker.start();
    let core = NesConsoleCore::new_empty(controller, speaker);
    let frame_ready = Arc::new(AtomicBool::new(false));
    let palette = match &pixel_format {
        PixelFormat::PaletteIndex { palette } => palette.clone(),
        PixelFormat::Rgba => Box::new([0u32; 256]),
    };
    let emu = EmuThread::spawn(
        Box::new(core),
        Arc::clone(&shared_fb),
        Arc::clone(&frame_ready),
        palette,
    );

    Ok(EmuCore::new(
        emu,
        render_profile,
        shared_fb,
        disp_fb,
        frame_ready,
    ))
}
