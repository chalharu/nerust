//! Runtime input events, values, and the GUI/emu state-buffer port.

use downcast_rs::Downcast;

use super::{
    ids::{AttachmentId, DigitalControlId},
    topology::ControlId,
};

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
            control: ControlId::Digital(self.control),
            value: match self.state {
                DigitalInputState::Pressed => InputValue::Digital(true),
                DigitalInputState::Released => InputValue::Digital(false),
            },
        }
    }
}

/// Unified runtime input event covering digital and analog controls.
///
/// [`DigitalInputEvent`] remains the narrow digital-only helper used by
/// touch overlays; convert it with [`DigitalInputEvent::to_general`] (or
/// `From`) when a single analog-capable path is needed. Prefer this type
/// for new code so analog controls do not need a parallel event enum.
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
        match (self.control, self.value) {
            (ControlId::Digital(control), InputValue::Digital(true)) => Some(
                DigitalInputEvent::new(self.attachment, control, DigitalInputState::Pressed),
            ),
            (ControlId::Digital(control), InputValue::Digital(false)) => Some(
                DigitalInputEvent::new(self.attachment, control, DigitalInputState::Released),
            ),
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
///
/// `PartialEq` follows IEEE-754 float semantics: `NaN != NaN`.
/// Buffers must not rely on `Analog` equality for state comparison;
/// use exact bit patterns (`to_bits`) or epsilon checks instead.
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
    ///
    /// `expected` is the static compile-time buffer name (`type_name::<Self>()`
    /// at the call site); `actual` is the runtime `type_name_of_val(other)`.
    /// Note the runtime name observes the trait object (`dyn InputStateBuffer`)
    /// rather than the erased concrete type, so it distinguishes "wrong object
    /// passed" from the exact concrete mismatch only coarsely; implementations
    /// must still refuse to partially apply state on mismatch.
    #[error("buffer type mismatch: expected {expected}, got {actual}")]
    MismatchedBuffer {
        expected: &'static str,
        actual: &'static str,
    },
}

/// GUI-side write abstraction for input state.
/// Emu side reads via `Any::downcast_ref`.
///
/// Field-index contract: `field` is a dense logical index in `0..N`
/// whose meaning is defined by the concrete buffer implementation
/// (documented alongside each core's buffer type). Out-of-range
/// indices must return [`BufferError::FieldNotFound`]; a value of the
/// wrong variant must return [`BufferError::UnsupportedFieldType`].
/// `clear` resets every field to its neutral (released) state, so a
/// freshly constructed buffer and a cleared buffer compare equal.
pub trait InputStateBuffer: std::fmt::Debug + Send + Downcast {
    /// Write one logical field.
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError>;
    /// Reset all fields to the neutral (released) state.
    /// Postcondition: the buffer equals a freshly constructed default.
    fn clear(&mut self);
    /// Copy absolute state from another buffer of the same concrete type.
    /// Returns [`BufferError::MismatchedBuffer`] when `other` has a
    /// different concrete type; implementations must not partially apply
    /// state on mismatch.
    fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError>;
}

downcast_rs::impl_downcast!(InputStateBuffer);

#[cfg(test)]
mod tests {
    use super::{AttachmentId, DigitalControlId, DigitalInputEvent, DigitalInputState};

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
}
