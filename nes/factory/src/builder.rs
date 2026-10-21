use nerust_core_traits::audio::AudioBackend;
use nerust_input_traits::InputReceiver;
use nerust_input_traits::{
    Controller, ControllerCollection, InputAssignments, InputSystemFactory, ProfileId,
    UnifiedFieldMap,
};
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_core::input_types::NesInputBuffer;
use nerust_render_filters::FilterTypeExt;
use nerust_render_traits::{VideoRenderProfile, filter::FilterType, logical::LogicalSize};
use nerust_system_traits::factory::{CoreParts, FactoryError, settings::FactorySettingsView};

/// Build controller devices from assignments and wrap the console in session parts.
/// Profile-to-device mapping lives here, next to the console construction it feeds.
pub(crate) fn create_core_and_adapter_with_assignments(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    assignments: &InputAssignments,
    input_factory: &crate::NesFactory,
) -> Result<CoreParts, FactoryError> {
    let controller_collection = controllers_for_assignments(assignments, input_factory)
        .map_err(|e| FactoryError::Create(e.to_string()))?;
    let resources = input_factory
        .create_split(&controller_collection)
        .map_err(|e| FactoryError::Create(e.to_string()))?;
    // Unified map travels end to end; no lossy projection.
    let (gui_input, emu_input, field_map) =
        nerust_emu_thread::InputSplit::typed_resources::<NesInputBuffer>(resources)
            .map_err(|e| FactoryError::Create(e.to_string()))?;
    create_core_and_adapter(
        view,
        speaker,
        Box::new(gui_input),
        emu_input,
        field_map,
        controller_collection,
    )
}

fn controllers_for_assignments(
    assignments: &InputAssignments,
    input_factory: &dyn InputSystemFactory,
) -> Result<ControllerCollection, nerust_input_traits::CreateSplitError> {
    let plan = assignments.placement_plan(input_factory)?;
    let mut devices: Vec<Option<Box<dyn Controller + Send>>> = Vec::with_capacity(plan.len());
    for placement in plan {
        let device: Option<Box<dyn Controller + Send>> = match placement {
            None => None,
            Some((pid, group)) if pid == ProfileId::new("nes.famicom") => Some(match group {
                0 => Box::new(nerust_nes_device::famicom_set::FamicomPadP1::new()),
                _ => Box::new(nerust_nes_device::famicom_set::FamicomPadP2::new()),
            }),
            Some((pid, _)) if pid == ProfileId::new("nes.standard_pad") => Some(Box::new(
                nerust_nes_device::standard_pad::StandardPad::new(0x1F),
            )),
            Some((controller, _)) => {
                return Err(nerust_input_traits::CreateSplitError::ControllerNotFound {
                    controller,
                });
            }
        };
        devices.push(device);
    }
    Ok(ControllerCollection::from_slots(devices))
}

pub(crate) fn create_core_and_adapter(
    view: &FactorySettingsView,
    speaker: Box<dyn AudioBackend>,
    gui_input: Box<dyn nerust_input_traits::GuiInputPort>,
    emu_input: InputReceiver<NesInputBuffer>,
    field_map: UnifiedFieldMap,
    controller_collection: ControllerCollection,
) -> Result<CoreParts, FactoryError> {
    let filter = crate::settings::filter_type_from_bytes(view.system_config.as_deref());

    let (render_profile, palette) = video_assets_for_filter(filter);
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

/// Video assets for one filter choice, without building a core.
/// Shared by core construction and the live swap, so they can never disagree.
pub(crate) fn video_assets_for_filter(filter: FilterType) -> (VideoRenderProfile, Box<[u32; 256]>) {
    let source_logical_size = LogicalSize {
        width: 256,
        height: 240,
    };
    let layout = filter.layout(source_logical_size);
    let assets = filter.palette_console_video_assets();
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

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_input_traits::{
        AttachmentId, ControllerHub, CreateSplitError, InputPorts, InputValue,
    };
    use nerust_nes_core::controller::NES_PORTS;

    const P1: AttachmentId = AttachmentId::new("nes.attachment.player1");
    const P2: AttachmentId = AttachmentId::new("nes.attachment.player2");

    fn pad() -> std::rc::Rc<dyn nerust_input_traits::ControllerProfile> {
        crate::NesFactory
            .resolve_controller("nes.standard_pad")
            .unwrap()
    }
    fn famicom() -> std::rc::Rc<dyn nerust_input_traits::ControllerProfile> {
        crate::NesFactory.resolve_controller("nes.famicom").unwrap()
    }

    #[test]
    fn player_two_only_keeps_player_one_unconnected() {
        // Both omitted P1 and an explicit empty P1 preserve physical slot 1.
        for slots in [vec![(P2, Some(pad()))], vec![(P2, Some(pad())), (P1, None)]] {
            let assignments = InputAssignments::new(slots);
            let mut devices =
                controllers_for_assignments(&assignments, &crate::NesFactory).unwrap();
            assert!(devices.get(0).is_none());
            assert!(devices.get(1).is_some());
            let resources = crate::NesFactory.create_split(&devices).unwrap();
            assert_eq!(resources.field_map().len(), 8);
            assert!(
                resources
                    .field_map()
                    .iter()
                    .all(|((attachment, _), _)| *attachment == P2)
            );
            let (_, make) = resources.into_parts();
            let mut state = make();
            state.set(8, InputValue::Digital(true)).unwrap();
            devices.sync_input(&state.downcast_ref::<NesInputBuffer>().unwrap().0);
            devices.write_strobe(1);
            assert_eq!(
                devices.read_port(&NES_PORTS[0]),
                nerust_input_traits::OpenBusReadResult::new(0, 0)
            );
            assert_eq!(devices.read_port(&NES_PORTS[1]).data & 1, 1);
        }
    }

    #[test]
    fn assignment_order_does_not_change_physical_placement() {
        let assignments = InputAssignments::new(vec![(P2, Some(pad())), (P1, Some(pad()))]);
        let devices = controllers_for_assignments(&assignments, &crate::NesFactory).unwrap();
        assert_eq!(
            crate::NesFactory
                .create_split(&devices)
                .unwrap()
                .field_map()
                .len(),
            16
        );
        assert!(
            controllers_for_assignments(
                &crate::NesFactory.default_assignments(),
                &crate::NesFactory
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_duplicate_unknown_incompatible_and_overlapping_assignments() {
        let factory = &crate::NesFactory;
        let build = |slots| controllers_for_assignments(&InputAssignments::new(slots), factory);
        assert!(matches!(
            build(vec![(P1, None), (P1, Some(pad()))]),
            Err(CreateSplitError::SlotConflict { .. })
        ));
        assert!(matches!(
            build(vec![(AttachmentId::new("unknown"), None)]),
            Err(CreateSplitError::SlotNotFound { .. })
        ));
        assert!(matches!(
            build(vec![(P2, Some(famicom()))]),
            Err(CreateSplitError::IncompatibleController { .. })
        ));
        for slots in [
            vec![(P1, Some(famicom())), (P2, Some(pad()))],
            vec![(P2, Some(pad())), (P1, Some(famicom()))],
        ] {
            assert!(matches!(
                build(slots),
                Err(CreateSplitError::SlotConflict { .. })
            ));
        }
        assert!(matches!(
            build(vec![(P1, Some(std::rc::Rc::new(Unknown)))]),
            Err(CreateSplitError::ControllerNotFound { .. })
        ));
    }

    #[derive(Debug)]
    struct Unknown;
    impl nerust_input_traits::ControllerProfile for Unknown {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("unknown")
        }
        fn label(&self) -> &'static str {
            "unknown"
        }
        fn port_sets(&self) -> &[nerust_input_traits::PortSet] {
            &[]
        }
        fn port_groups(&self) -> &[&[nerust_input_traits::ControlInfo]] {
            &[]
        }
    }
}
