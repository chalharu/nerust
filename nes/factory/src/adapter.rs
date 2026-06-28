use std::sync::Arc;

use nerust_core_traits::input::{InputError, InputStatePersistence, SystemInputAdapter};
use nerust_input_traits::DigitalInputEvent;
use nerust_nes_controller::{
    codec::{decode_input_state, encode_input_state},
    nes_input_cell::NesInputCell,
    persisted::digital_event_from_persisted_ids,
};

use crate::input_state::NesInputState;

#[derive(Debug)]
pub(crate) struct NesAdapter {
    input: NesInputState,
    cell: Arc<NesInputCell>,
}

impl NesAdapter {
    pub fn new(cell: Arc<NesInputCell>) -> Self {
        Self {
            input: NesInputState::default(),
            cell,
        }
    }
}

impl SystemInputAdapter for NesAdapter {
    fn apply_event(&mut self, event: DigitalInputEvent) {
        self.input.handle_input(event);
        let frame = self.input.current_frame();
        self.cell.store(
            frame.player_one.bits(),
            frame.player_two.bits(),
            frame.microphone,
        );
    }

    fn clear(&mut self) {
        let _ = self.input.clear_current_frame();
        self.cell.store(0, 0, false);
    }

    fn decode_persisted_input(
        &self,
        attachment_id: &str,
        control_id: &str,
        pressed: bool,
    ) -> Option<DigitalInputEvent> {
        digital_event_from_persisted_ids(attachment_id, control_id, pressed)
    }
}

impl InputStatePersistence for NesAdapter {
    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), InputError> {
        let frame = decode_input_state(bytes).map_err(|e| InputError::Decode(e.to_string()))?;
        self.input.sync_from_frame(frame);
        self.cell.store(
            frame.player_one.bits(),
            frame.player_two.bits(),
            frame.microphone,
        );
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, InputError> {
        encode_input_state(self.input.current_frame())
            .map_err(|e| InputError::Encode(e.to_string()))
    }
}
