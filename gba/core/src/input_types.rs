use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

#[derive(Debug, Clone, Default)]
pub struct GbaInputBuffer {
    // Phase 2 で実装
}

impl InputStateBuffer for GbaInputBuffer {
    fn set(&mut self, _field: usize, _value: InputValue) -> Result<(), BufferError> {
        // Phase 2 で実装
        Ok(())
    }

    fn clear(&mut self) {
        // Phase 2 で実装
    }

    fn copy_state(&mut self, _other: &dyn InputStateBuffer) {
        // Phase 2 で実装
    }
}
