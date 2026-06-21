use crate::input_state::NesInputState;
use nerust_contract_core::input::SystemInputAdapter;
use nerust_input_nes_runtime::codec::{decode_input_state, encode_input_state};
use nerust_input_nes_runtime::nes_input_cell::NesInputCell;
use nerust_input_nes_runtime::persisted::digital_event_from_persisted_ids;
use nerust_input_schema::DigitalInputEvent;
use std::sync::Arc;

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

    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let frame = decode_input_state(bytes).map_err(|error| error.to_string())?;
        self.input.sync_from_frame(frame);
        self.cell.store(
            frame.player_one.bits(),
            frame.player_two.bits(),
            frame.microphone,
        );
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String> {
        encode_input_state(self.input.current_frame()).map_err(|error| error.to_string())
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
