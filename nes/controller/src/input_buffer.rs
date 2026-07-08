use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// `[u8; 3]` implements InputStateBuffer for NES input.
///
/// Field layout:
///   0-7:   P1 (A, B, Select, Start, Up, Down, Left, Right)
///   8-15:  P2 (A, B, Select, Start, Up, Down, Left, Right)
///   16:    Microphone
impl InputStateBuffer for [u8; 3] {
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        match value {
            InputValue::Digital(pressed) => match field {
                0..=7 => {
                    let mask = 1 << field;
                    if pressed {
                        self[0] |= mask;
                    } else {
                        self[0] &= !mask;
                    }
                    Ok(())
                }
                8..=15 => {
                    let mask = 1 << (field - 8);
                    if pressed {
                        self[1] |= mask;
                    } else {
                        self[1] &= !mask;
                    }
                    Ok(())
                }
                16 => {
                    if pressed {
                        self[2] = 1;
                    } else {
                        self[2] = 0;
                    }
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
        *self = [0; 3];
    }
}
