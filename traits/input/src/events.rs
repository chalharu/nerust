//! Runtime input events, values, and the GUI/emu state-buffer port.

use downcast_rs::Downcast;

use super::ids::{AttachmentId, ControlId, DigitalControlId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitalInputState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitalInputEvent {
    pub attachment: AttachmentId,
    pub control: DigitalControlId,
    pub state: DigitalInputState,
}

impl DigitalInputEvent {
    pub const fn new(
        attachment: AttachmentId,
        control: DigitalControlId,
        state: DigitalInputState,
    ) -> Self {
        Self {
            attachment,
            control,
            state,
        }
    }

    pub const fn pressed(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Pressed)
    }

    pub const fn released(attachment: AttachmentId, control: DigitalControlId) -> Self {
        Self::new(attachment, control, DigitalInputState::Released)
    }

    pub const fn is_pressed(self) -> bool {
        matches!(self.state, DigitalInputState::Pressed)
    }

    /// Lift into the unified [`InputEvent`] path (digital only).
    pub const fn to_general(self) -> InputEvent {
        InputEvent {
            attachment: self.attachment,
            control: ControlId::digital(self.control),
            value: match self.state {
                DigitalInputState::Pressed => InputValue::Digital(true),
                DigitalInputState::Released => InputValue::Digital(false),
            },
        }
    }
}

/// Unified runtime input event covering digital and analog controls.
/// Prefer this for new code; convert via [`DigitalInputEvent::to_general`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputEvent {
    pub attachment: AttachmentId,
    pub control: ControlId,
    pub value: InputValue,
}

impl InputEvent {
    pub const fn new(attachment: AttachmentId, control: ControlId, value: InputValue) -> Self {
        Self {
            attachment,
            control,
            value,
        }
    }

    /// Narrow back to a digital event when the control and value are digital.
    pub fn as_digital(self) -> Option<DigitalInputEvent> {
        let control = self.control.as_digital()?;
        match self.value {
            InputValue::Digital(true) => Some(DigitalInputEvent::new(
                self.attachment,
                control,
                DigitalInputState::Pressed,
            )),
            InputValue::Digital(false) => Some(DigitalInputEvent::new(
                self.attachment,
                control,
                DigitalInputState::Released,
            )),
            _ => None,
        }
    }
}

impl From<DigitalInputEvent> for InputEvent {
    fn from(event: DigitalInputEvent) -> Self {
        event.to_general()
    }
}

/// Result of a CPU bus read — data bits and which bits are valid.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub const fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}

/// Values that can be written to an `InputStateBuffer` field.
/// `PartialEq` follows IEEE-754 float semantics: `NaN != NaN`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputValue {
    Digital(bool),
    Analog(f32),
    Position { x: f64, y: f64 },
}

/// Errors from `InputStateBuffer` operations.
#[derive(Debug, thiserror::Error)]
pub enum BufferError {
    #[error("field {field} not found")]
    FieldNotFound { field: usize },
    #[error("field {field} does not support value type {expected}")]
    UnsupportedFieldType {
        field: usize,
        expected: &'static str,
    },
    /// Buffer concrete-type mismatch on `copy_state`.
    /// `expected` is the compile-time name; `actual` is the runtime name.
    #[error("buffer type mismatch: expected {expected}, got {actual}")]
    MismatchedBuffer {
        expected: &'static str,
        actual: &'static str,
    },
}

/// Typed absolute field index for input buffers. New code passes this;
/// the `usize` overloads stay for the frozen GUI port surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(usize);

impl FieldId {
    /// Absolute field index in `0..N`.
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Raw index for the `usize` port surface.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// GUI-side write abstraction for input state.
/// Emu side reads via `Any::downcast_ref`; `field` is a dense index in `0..N`.
pub trait InputStateBuffer: std::fmt::Debug + Send + Downcast {
    /// Write one logical field.
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError>;
    /// Typed write; defaults to [`Self::set`].
    fn set_field(&mut self, field: FieldId, value: InputValue) -> Result<(), BufferError> {
        self.set(field.index(), value)
    }
    /// Reset all fields to the neutral (released) state.
    /// Postcondition: the buffer equals a freshly constructed default.
    fn clear(&mut self);
    /// Copy absolute state; mismatched types refuse without partial apply.
    /// Partial merge is absent: rotation resurrects superseded bytes.
    fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError>;
}

downcast_rs::impl_downcast!(InputStateBuffer);

/// Core-facing input receiver, independent of any thread or transport.
pub trait InputReceiver: Send {
    /// Refresh once at frame start and borrow the latest complete snapshot.
    /// Without new input, retain the previous state. The snapshot stays valid
    /// for the borrow; implementations should avoid per-frame allocation.
    fn receive(&mut self) -> &dyn InputStateBuffer;
}

/// A local buffer is also a receiver with a fixed snapshot (no transport).
impl<B: InputStateBuffer> InputReceiver for B {
    fn receive(&mut self) -> &dyn InputStateBuffer {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AttachmentId, BufferError, DigitalControlId, DigitalInputEvent, DigitalInputState,
        InputStateBuffer, InputValue,
    };

    // Copy-state contract shared by all buffers.
    fn assert_copy_contract<B>(mut fresh: B, staged: B, foreign: &dyn InputStateBuffer)
    where
        B: InputStateBuffer + PartialEq + std::fmt::Debug,
    {
        fresh.copy_state(&staged).unwrap();
        assert_eq!(fresh, staged);
        assert!(fresh.copy_state(foreign).is_err());
    }

    #[derive(Debug)]
    struct Foreign;
    impl InputStateBuffer for Foreign {
        fn set(&mut self, _f: usize, _v: InputValue) -> Result<(), BufferError> {
            Ok(())
        }
        fn clear(&mut self) {}
        fn copy_state(&mut self, _: &dyn InputStateBuffer) -> Result<(), BufferError> {
            Err(BufferError::MismatchedBuffer {
                expected: "Foreign",
                actual: "other",
            })
        }
    }

    #[test]
    fn local_buffer_receives_without_transport() {
        let mut receiver: Box<dyn super::InputReceiver> = Box::new(Foreign);
        assert!(receiver.receive().downcast_ref::<Foreign>().is_some());
        assert!(receiver.receive().downcast_ref::<Foreign>().is_some());
    }

    #[test]
    fn digital_input_event_helpers_preserve_state() {
        let attachment = AttachmentId::new("test.pad1");
        let control = DigitalControlId::new("test.a");

        assert!(DigitalInputEvent::pressed(attachment, control).is_pressed());
        assert_eq!(
            DigitalInputEvent::released(attachment, control).state,
            DigitalInputState::Released
        );
    }

    #[test]
    fn general_event_round_trips_digital() {
        use super::{ControlId, InputEvent, InputValue};
        use crate::AnalogControlId;
        let attachment = AttachmentId::new("test.pad1");
        let digital = DigitalControlId::new("test.a");
        let pressed = DigitalInputEvent::pressed(attachment, digital);
        let general = pressed.to_general();
        assert_eq!(general.value, InputValue::Digital(true));
        let back = general.as_digital().expect("digital round-trip");
        assert_eq!(back, pressed);
        let analog = InputEvent::new(
            attachment,
            ControlId::analog(AnalogControlId::new("test.x")),
            InputValue::Analog(0.5),
        );
        assert!(analog.as_digital().is_none());
        let foreign = InputEvent::new(
            attachment,
            ControlId::analog(AnalogControlId::new("test.x")),
            InputValue::Digital(true),
        );
        assert!(foreign.as_digital().is_none());
        let from: InputEvent = pressed.into();
        assert_eq!(from, general);
    }

    #[test]
    fn typed_field_id_round_trips_through_set() {
        use super::FieldId;
        #[derive(Debug, Default, PartialEq)]
        struct Bit {
            pressed: bool,
        }
        impl InputStateBuffer for Bit {
            fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
                if field != 0 {
                    return Err(BufferError::FieldNotFound { field });
                }
                let InputValue::Digital(pressed) = value else {
                    return Err(BufferError::UnsupportedFieldType {
                        field,
                        expected: "digital",
                    });
                };
                self.pressed = pressed;
                Ok(())
            }
            fn clear(&mut self) {
                self.pressed = false;
            }
            fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
                let Some(source) = other.downcast_ref::<Self>() else {
                    return Err(BufferError::MismatchedBuffer {
                        expected: "Bit",
                        actual: "test",
                    });
                };
                self.pressed = source.pressed;
                Ok(())
            }
        }
        let mut bit = Bit::default();
        // Typed write delegates to the `usize` port.
        bit.set_field(FieldId::new(0), InputValue::Digital(true))
            .unwrap();
        assert!(bit.pressed);
        assert!(
            bit.set_field(FieldId::new(9), InputValue::Digital(true))
                .is_err()
        );
        assert_eq!(FieldId::new(3).index(), 3);
        // Copy-state contract shared by all buffers.
        assert_copy_contract(Bit::default(), bit, &Foreign);
    }

    #[test]
    fn buffer_error_display_names_fields() {
        use super::BufferError;
        let missing = BufferError::MismatchedBuffer {
            expected: "A",
            actual: "B",
        };
        assert!(missing.to_string().contains('A'));
        let unsupported = BufferError::UnsupportedFieldType {
            field: 3,
            expected: "digital",
        };
        assert!(unsupported.to_string().contains('3'));
        let absent = BufferError::FieldNotFound { field: 9 };
        assert!(absent.to_string().contains('9'));
        let result = super::OpenBusReadResult::new(1, 2);
        assert_eq!((result.data, result.mask), (1, 2));
    }
}
