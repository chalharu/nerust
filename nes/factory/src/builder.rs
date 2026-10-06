use std::collections::HashMap;

use nerust_core_traits::{
    audio::AudioBackend,
    factory::{CoreParts, FactoryError, settings::FactorySettingsView},
};
use nerust_input_traits::{
    AttachmentId, Controller, ControllerCollection, DigitalControlId, EmuInput, GuiInput,
    InputAssignments, InputSystemFactory, ProfileId,
};
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_render_filters::FilterTypeExt;
use nerust_render_traits::{VideoRenderProfile, filter::FilterType, logical::LogicalSize};

/// Build controller devices from assignments, split input, and wrap
/// the console in session parts. Profile-to-device mapping lives here
/// (not in `lib.rs`), next to the console construction it feeds.
pub(crate) fn create_core_and_adapter_with_assignments(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    assignments: &InputAssignments,
    input_factory: &dyn InputSystemFactory,
) -> Result<CoreParts, FactoryError> {
    // Build controller devices per occupied port.
    let mut devices: Vec<Box<dyn Controller + Send>> = Vec::new();
    for (_, ctrl_opt) in &assignments.slots {
        let profile = match ctrl_opt {
            Some(p) => p,
            None => continue,
        };
        let pid = profile.profile_id();
        if pid == ProfileId::new("nes.famicom") {
            devices.push(Box::new(nerust_nes_device::famicom_set::FamicomPadP1::new()));
            devices.push(Box::new(nerust_nes_device::famicom_set::FamicomPadP2::new()));
        } else if pid == ProfileId::new("nes.standard_pad") {
            devices.push(Box::new(nerust_nes_device::standard_pad::StandardPad::new(
                0x1F,
            )));
        }
    }
    let controller_collection = ControllerCollection::new(devices);
    let resources = input_factory
        .create_split(&controller_collection)
        .map_err(|e| FactoryError::Create(e.to_string()))?;
    let gui_input = GuiInput::from_split(&resources.split);
    let emu_input = EmuInput::from_split(&resources.split);
    create_core_and_adapter(
        view,
        speaker,
        gui_input,
        emu_input,
        resources.field_map,
        controller_collection,
    )
}

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    gui_input: GuiInput,
    emu_input: EmuInput,
    field_map: HashMap<(AttachmentId, DigitalControlId), usize>,
    controller_collection: ControllerCollection,
) -> Result<CoreParts, FactoryError> {
    let filter = crate::settings::filter_type_from_bytes(view.system_config.as_deref());

    let (render_profile, palette) = compute_render_profile(filter);
    let mut speaker = speaker;
    speaker.start();
    let core = NesConsoleCore::new_empty(controller_collection, emu_input);
    Ok(CoreParts {
        core: Box::new(core),
        audio: speaker,
        gui_input,
        field_map,
        render_profile,
        palette,
        host_peripherals: Default::default(),
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
        frame_format: nerust_render_traits::VideoFrameFormat::Palette,
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
