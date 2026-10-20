use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// NES 入力バッファ。P1(1byte) + P2(1byte) + mic(1byte) の 3 bytes。
/// Field 0-7: P1、8-15: P2（A/B/Select/Start/Up/Down/Left/Right）、16: mic。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NesInputBuffer(pub [u8; 3]);

impl InputStateBuffer for NesInputBuffer {
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        match value {
            InputValue::Digital(pressed) => match field {
                0..=7 => {
                    let mask = 1 << field;
                    if pressed {
                        self.0[0] |= mask;
                    } else {
                        self.0[0] &= !mask;
                    }
                    Ok(())
                }
                8..=15 => {
                    let mask = 1 << (field - 8);
                    if pressed {
                        self.0[1] |= mask;
                    } else {
                        self.0[1] &= !mask;
                    }
                    Ok(())
                }
                16 => {
                    self.0[2] = if pressed { 1 } else { 0 };
                    Ok(())
                }
                _ => Err(BufferError::FieldNotFound { field }),
            },
            _ => Err(BufferError::UnsupportedFieldType {
                field,
                expected: "digital",
            }),
        }
    }

    fn clear(&mut self) {
        self.0 = [0; 3];
    }

    fn copy_state(
        &mut self,
        other: &dyn nerust_input_traits::InputStateBuffer,
    ) -> Result<(), nerust_input_traits::BufferError> {
        if let Some(src) = other.downcast_ref::<NesInputBuffer>() {
            self.0 = src.0;
            Ok(())
        } else {
            Err(nerust_input_traits::BufferError::MismatchedBuffer {
                expected: std::any::type_name::<Self>(),
                actual: std::any::type_name_of_val(other),
            })
        }
    }

    /// Merge only staged bits: untouched bytes survive without a full copy.
    fn copy_dirty_from(
        &mut self,
        other: &dyn nerust_input_traits::InputStateBuffer,
        dirty: &[usize],
    ) -> Result<(), nerust_input_traits::BufferError> {
        use nerust_input_traits::BufferError;
        let Some(src) = other.downcast_ref::<NesInputBuffer>() else {
            return Err(BufferError::MismatchedBuffer {
                expected: std::any::type_name::<Self>(),
                actual: std::any::type_name_of_val(other),
            });
        };
        for &field in dirty {
            match field {
                0..=7 => {
                    let mask = 1 << field;
                    self.0[0] = (self.0[0] & !mask) | (src.0[0] & mask);
                }
                8..=15 => {
                    let mask = 1 << (field - 8);
                    self.0[1] = (self.0[1] & !mask) | (src.0[1] & mask);
                }
                16 => self.0[2] = src.0[2],
                _ => return Err(BufferError::FieldNotFound { field }),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::NesInputBuffer;
    use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

    #[test]
    fn dirty_merge_keeps_unstaged_bytes() {
        use nerust_input_traits::InputValue;
        let mut staged = NesInputBuffer::default();
        staged.set(0, InputValue::Digital(true)).unwrap();
        staged.set(16, InputValue::Digital(true)).unwrap();
        let mut working = NesInputBuffer::default();
        working.set(8, InputValue::Digital(true)).unwrap();
        // Only staged fields merge; working P2 bit survives.
        working.copy_dirty_from(&staged, &[0, 16]).unwrap();
        assert_eq!(working.0, [0x01, 0x01, 1]);
        // Empty list merges nothing; unknown fields refuse.
        working.copy_dirty_from(&staged, &[]).unwrap();
        assert!(working.copy_dirty_from(&staged, &[17]).is_err());
    }

    #[test]
    fn p1_and_p2_fields_set_and_clear_bits() {
        let mut buf = NesInputBuffer::default();
        buf.set(0, InputValue::Digital(true)).unwrap();
        buf.set(9, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0, [0x01, 0x02, 0]);
        buf.set(0, InputValue::Digital(false)).unwrap();
        assert_eq!(buf.0[0], 0);
        buf.set(16, InputValue::Digital(true)).unwrap();
        assert_eq!(buf.0[2], 1);
        buf.clear();
        assert_eq!(buf, NesInputBuffer::default());
    }

    #[test]
    fn rejects_bad_field_and_bad_value() {
        let mut buf = NesInputBuffer::default();
        assert!(matches!(
            buf.set(17, InputValue::Digital(true)),
            Err(BufferError::FieldNotFound { field: 17 })
        ));
        assert!(matches!(
            buf.set(0, InputValue::Analog(1.0)),
            Err(BufferError::UnsupportedFieldType { .. })
        ));
        assert!(matches!(
            buf.set(0, InputValue::Position { x: 0.0, y: 0.0 }),
            Err(BufferError::UnsupportedFieldType { .. })
        ));
    }

    #[test]
    fn copy_state_moves_bytes_or_refuses_foreign() {
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
        let mut dst = NesInputBuffer::default();
        let src = NesInputBuffer([0xAA, 0x55, 1]);
        dst.copy_state(&src).unwrap();
        assert_eq!(dst.0, [0xAA, 0x55, 1]);
        assert!(dst.copy_state(&Foreign).is_err());
    }
}
