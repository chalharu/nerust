use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// Active-low state for the eight Game Boy buttons.
/// Bits 0-3 are A/B/Select/Start, bits 4-7 are Right/Left/Up/Down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GbcInputBuffer(pub u8);

impl Default for GbcInputBuffer {
    fn default() -> Self {
        Self(0xFF)
    }
}

impl InputStateBuffer for GbcInputBuffer {
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        let InputValue::Digital(pressed) = value else {
            return Err(BufferError::UnsupportedFieldType {
                field,
                expected: "digital",
            });
        };
        if field >= 8 {
            return Err(BufferError::FieldNotFound { field });
        }
        let mask = 1 << field;
        if pressed {
            self.0 &= !mask;
        } else {
            self.0 |= mask;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.0 = 0xFF;
    }

    fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
        if let Some(other) = other.downcast_ref::<Self>() {
            self.0 = other.0;
            Ok(())
        } else {
            Err(BufferError::MismatchedBuffer {
                expected: std::any::type_name::<Self>(),
                actual: std::any::type_name_of_val(other),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Copy-state contract shared by all buffers.
    fn assert_copy_contract<B>(mut fresh: B, staged: B, foreign: &dyn InputStateBuffer)
    where
        B: InputStateBuffer + PartialEq + std::fmt::Debug,
    {
        fresh.copy_state(&staged).unwrap();
        assert_eq!(fresh, staged);
        assert!(fresh.copy_state(foreign).is_err());
    }

    #[test]
    fn press_is_active_low_and_clear_releases_all() {
        let mut input = GbcInputBuffer::default();
        input.set(0, InputValue::Digital(true)).unwrap();
        input.set(7, InputValue::Digital(true)).unwrap();
        assert_eq!(input.0, 0x7E);

        input.set(0, InputValue::Digital(false)).unwrap();
        assert_eq!(input.0, 0x7F);
        input.clear();
        assert_eq!(input, GbcInputBuffer::default());
    }

    #[test]
    fn rejects_invalid_field_and_value_type() {
        let mut input = GbcInputBuffer::default();
        assert!(matches!(
            input.set(8, InputValue::Digital(true)),
            Err(BufferError::FieldNotFound { field: 8 })
        ));
        assert!(matches!(
            input.set(0, InputValue::Analog(1.0)),
            Err(BufferError::UnsupportedFieldType { field: 0, .. })
        ));
    }

    #[test]
    fn contract_full_copy_and_mismatch_refusal() {
        #[derive(Debug, Default)]
        struct Foreign;
        impl InputStateBuffer for Foreign {
            fn set(&mut self, _: usize, _: InputValue) -> Result<(), BufferError> {
                Ok(())
            }
            fn clear(&mut self) {}
            fn copy_state(&mut self, _: &dyn InputStateBuffer) -> Result<(), BufferError> {
                Ok(())
            }
        }
        let mut staged = GbcInputBuffer::default();
        staged.set(0, InputValue::Digital(true)).unwrap();
        staged.set(7, InputValue::Digital(true)).unwrap();
        let mut working = GbcInputBuffer::default();
        working.set(3, InputValue::Digital(true)).unwrap();
        // Full copy overwrites everything: rotation cannot resurrect bits.
        assert_copy_contract(working, staged, &Foreign);
        let mut raw = GbcInputBuffer::default();
        raw.copy_state(&GbcInputBuffer(0x3C)).unwrap();
        assert_eq!(raw.0, 0x3C);
    }
}
