use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// Active-low state for the ten GBA buttons (GBATEK KEYINPUT 4000130h).
/// Upper 6 bits are unused and always 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GbaInputBuffer(pub u16);

impl Default for GbaInputBuffer {
    fn default() -> Self {
        Self(0x03FF)
    }
}

impl InputStateBuffer for GbaInputBuffer {
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        let InputValue::Digital(pressed) = value else {
            return Err(BufferError::UnsupportedFieldType {
                field,
                expected: "digital",
            });
        };
        if field >= 10 {
            return Err(BufferError::FieldNotFound { field });
        }
        let mask = 1u16 << field;
        if pressed {
            self.0 &= !mask;
        } else {
            self.0 |= mask;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.0 = 0x03FF;
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
        assert_eq!(GbaInputBuffer::default().0, 0x03FF);
        let mut buf = GbaInputBuffer::default();
        buf.set(0, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0, 0x03FE);
        buf.set(0, InputValue::Digital(false)).unwrap();
        assert_eq!(buf.0, 0x03FF);
        buf.set(8, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0 & (1 << 8), 0);
        buf.clear();
        assert_eq!(buf.0, 0x03FF);
    }

    #[test]
    fn rejects_bad_field_and_bad_value() {
        let mut buf = GbaInputBuffer::default();
        assert!(matches!(
            buf.set(10, InputValue::Digital(true)),
            Err(BufferError::FieldNotFound { field: 10 })
        ));
        assert!(matches!(
            buf.set(0, InputValue::Analog(0.5)),
            Err(BufferError::UnsupportedFieldType { .. })
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
        let mut staged = GbaInputBuffer::default();
        staged.set(0, InputValue::Digital(true)).unwrap();
        staged.set(9, InputValue::Digital(true)).unwrap();
        let mut working = GbaInputBuffer::default();
        working.set(8, InputValue::Digital(true)).unwrap();
        // Full copy overwrites everything: rotation cannot resurrect bits.
        assert_copy_contract(working, staged, &Foreign);
        let mut raw = GbaInputBuffer(0x0200);
        raw.copy_state(&GbaInputBuffer(0x0155)).unwrap();
        assert_eq!(raw.0, 0x0155);
    }
}
