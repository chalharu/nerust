use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// NES 入力バッファ。P1(1byte) + P2(1byte) + mic(1byte) の 3 bytes。
///
/// Field layout:
///   0-7:   P1 (A, B, Select, Start, Up, Down, Left, Right)
///   8-15:  P2 (A, B, Select, Start, Up, Down, Left, Right)
///   16:    Microphone
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

    fn copy_state(&mut self, other: &dyn nerust_input_traits::InputStateBuffer) {
        if let Some(src) = other.downcast_ref::<NesInputBuffer>() {
            self.0 = src.0;
        }
    }
}
