use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// Active-low state for the eight Game Boy buttons.
///
/// Bits 0-3 are A, B, Select, Start. Bits 4-7 are Right, Left, Up, Down.
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

    fn copy_state(&mut self, other: &dyn InputStateBuffer) {
        if let Some(other) = other.downcast_ref::<Self>() {
            self.0 = other.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_digital_fields_as_active_low_bits() {
        let mut input = GbcInputBuffer::default();
        input.set(0, InputValue::Digital(true)).unwrap();
        input.set(7, InputValue::Digital(true)).unwrap();
        assert_eq!(input.0, 0x7E);

        input.set(0, InputValue::Digital(false)).unwrap();
        assert_eq!(input.0, 0x7F);
    }

    #[test]
    fn clear_releases_all_buttons() {
        let mut input = GbcInputBuffer(0);
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
}
