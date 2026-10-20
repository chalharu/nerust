use std::rc::Rc;

use nerust_input_traits::{
    AttachmentId, ControllerCollection, ControllerProfile, CreateSplitError, InputAssignments,
    InputPorts, InputResources, InputSystemFactory, ProfileId, SlotInfo,
};
use nerust_nes_core::input_types::NesInputBuffer;

impl InputPorts for crate::NesFactory {
    fn slots(&self) -> &[SlotInfo] {
        static SLOTS: &[SlotInfo] = &[
            SlotInfo {
                id: AttachmentId::new("nes.attachment.player1"),
                label: "Player 1",
            },
            SlotInfo {
                id: AttachmentId::new("nes.attachment.player2"),
                label: "Player 2",
            },
        ];
        SLOTS
    }
    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
        Box::new(nerust_nes_device::nes_device_controller_profiles())
    }
}

impl InputSystemFactory for crate::NesFactory {
    fn default_assignments(&self) -> InputAssignments {
        let famicom = self
            .controllers()
            .find(|p| p.profile_id() == ProfileId::new("nes.famicom"));
        InputAssignments::new(vec![
            (AttachmentId::new("nes.attachment.player1"), famicom),
            (AttachmentId::new("nes.attachment.player2"), None),
        ])
    }

    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError> {
        use nerust_nes_core::controller::NES_PORTS;

        let mut field_map = std::collections::HashMap::new();
        for (idx, port) in NES_PORTS.iter().enumerate() {
            let Some(dev) = controllers.get(idx) else {
                continue;
            };
            for entry in dev.field_entries(port) {
                field_map.insert((entry.attachment, entry.control), entry.field);
            }
        }
        if field_map.is_empty() {
            return Err(CreateSplitError::ControllerNotFound {
                controller: ProfileId::new("nes.standard_pad"),
            });
        }

        Ok(InputResources::new(
            field_map,
            Box::new(|| Box::<NesInputBuffer>::default()),
        ))
    }
}
