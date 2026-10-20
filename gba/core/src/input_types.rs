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

    /// Merge only staged bits: the staged word keeps working state,
    /// untouched bits survive without a full copy.
    fn copy_dirty_from(
        &mut self,
        other: &dyn InputStateBuffer,
        dirty: &[usize],
    ) -> Result<(), BufferError> {
        let Some(source) = other.downcast_ref::<Self>() else {
            return Err(BufferError::MismatchedBuffer {
                expected: std::any::type_name::<Self>(),
                actual: std::any::type_name_of_val(other),
            });
        };
        for &field in dirty {
            if field >= 10 {
                return Err(BufferError::FieldNotFound { field });
            }
            let mask = 1u16 << field;
            self.0 = (self.0 & !mask) | (source.0 & mask);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_released() {
        assert_eq!(GbaInputBuffer::default().0, 0x03FF);
    }

    #[test]
    fn set_a_press_clears_bit0() {
        let mut buf = GbaInputBuffer::default();
        buf.set(0, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0, 0x03FE);
    }

    #[test]
    fn set_then_release_restores() {
        let mut buf = GbaInputBuffer::default();
        buf.set(8, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0 & (1 << 8), 0);
        buf.set(8, InputValue::Digital(false)).unwrap();
        assert_eq!(buf.0, 0x03FF);
    }

    #[test]
    fn dirty_merge_keeps_unstaged_bits() {
        let mut staged = GbaInputBuffer::default();
        staged.set(0, InputValue::Digital(true)).unwrap();
        staged.set(9, InputValue::Digital(true)).unwrap();
        let mut working = GbaInputBuffer::default();
        working.set(8, InputValue::Digital(true)).unwrap();
        // Only staged fields merge; working bit 8 survives.
        working.copy_dirty_from(&staged, &[0, 9]).unwrap();
        assert_eq!(working.0 & 1, 0);
        assert_eq!(working.0 & (1 << 9), 0);
        assert_eq!(working.0 & (1 << 8), 0);
        // Empty list merges nothing; unknown fields refuse.
        working.copy_dirty_from(&staged, &[]).unwrap();
        assert!(working.copy_dirty_from(&staged, &[10]).is_err());
    }

    #[test]
    fn field_out_of_range_returns_error() {
        let mut buf = GbaInputBuffer::default();
        assert!(matches!(
            buf.set(10, InputValue::Digital(true)),
            Err(BufferError::FieldNotFound { field: 10 })
        ));
    }

    #[test]
    fn analog_returns_unsupported() {
        let mut buf = GbaInputBuffer::default();
        assert!(matches!(
            buf.set(0, InputValue::Analog(0.5)),
            Err(BufferError::UnsupportedFieldType { .. })
        ));
    }

    #[test]
    fn copy_state_copies_u16() {
        let mut a = GbaInputBuffer(0x0200);
        let b = GbaInputBuffer(0x0155);
        a.copy_state(&b).unwrap();
        assert_eq!(a.0, 0x0155);
    }

    #[test]
    fn clear_resets_to_default() {
        let mut buf = GbaInputBuffer(0x0000);
        buf.clear();
        assert_eq!(buf.0, 0x03FF);
    }

    #[test]
    fn copy_state_refuses_foreign_type() {
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
        let mut buf = GbaInputBuffer::default();
        assert!(buf.copy_state(&Foreign).is_err());
    }
}
