bitflags::bitflags! {
    #[derive(
        serde::Serialize,
        serde::Deserialize,
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
    )]
    pub struct Buttons: u8 {
        const A =      0b0000_0001;
        const B =      0b0000_0010;
        const SELECT = 0b0000_0100;
        const START =  0b0000_1000;
        const UP =     0b0001_0000;
        const DOWN =   0b0010_0000;
        const LEFT =   0b0100_0000;
        const RIGHT =  0b1000_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NesInputFrame {
    pub player_one: Buttons,
    pub player_two: Buttons,
    pub microphone: bool,
}

impl Default for NesInputFrame {
    fn default() -> Self {
        Self {
            player_one: Buttons::empty(),
            player_two: Buttons::empty(),
            microphone: false,
        }
    }
}

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
        let any: &dyn std::any::Any = other;
        if let Some(src) = any.downcast_ref::<NesInputBuffer>() {
            self.0 = src.0;
        }
    }
}
