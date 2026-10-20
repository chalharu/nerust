//! PhysicalKeyTracker: transient physical-controller presses.
//! Multi-controller chords share one press; releases fan out per key.

use std::collections::HashSet;

use nerust_core_traits::touch::{TouchControlRole, TouchOverlayModel};
use nerust_gui_shell::session::SessionHandle;
use nerust_input_traits::{AbstractKey, AttachmentId, DigitalControlId};

/// Pressed physical keys as `(device_id, key)` pairs.
/// Chords press once; the last release wins.
pub(super) struct PhysicalKeyTracker {
    pressed: HashSet<(i32, AbstractKey)>,
}

impl PhysicalKeyTracker {
    pub(super) fn new() -> Self {
        Self {
            pressed: HashSet::new(),
        }
    }

    /// Forget transient presses on focus loss without touching the
    /// emulator (the session already cleared its own input).
    pub(super) fn forget(&mut self) {
        self.pressed.clear();
    }

    /// Release every physically held button before forgetting it.
    /// The overlay model is built once and shared by every key.
    pub(super) fn release_all(&mut self, session: &mut SessionHandle, model: &TouchOverlayModel) {
        for (_, key) in std::mem::take(&mut self.pressed) {
            if let Some((attachment, control)) = Self::control_for_key(model, key) {
                session.apply_input_event(nerust_input_traits::DigitalInputEvent::released(
                    attachment, control,
                ));
            }
        }
    }

    /// Role lookup against a prebuilt overlay model.
    /// Callers build the model once per pass/event and share it across keys.
    fn control_for_key(
        model: &TouchOverlayModel,
        key: AbstractKey,
    ) -> Option<(AttachmentId, DigitalControlId)> {
        let role = match key {
            AbstractKey::Button1 => TouchControlRole::FaceButton1,
            AbstractKey::Button2 => TouchControlRole::FaceButton2,
            AbstractKey::Button5 => TouchControlRole::LeftShoulder,
            AbstractKey::Button6 => TouchControlRole::RightShoulder,
            AbstractKey::Start => TouchControlRole::Start,
            AbstractKey::Select => TouchControlRole::Select,
            AbstractKey::DpadUp => TouchControlRole::DpadUp,
            AbstractKey::DpadDown => TouchControlRole::DpadDown,
            AbstractKey::DpadLeft => TouchControlRole::DpadLeft,
            AbstractKey::DpadRight => TouchControlRole::DpadRight,
            _ => return None,
        };
        model
            .controls
            .iter()
            .find(|control| control.role == role)
            .map(|control| (control.attachment_id, control.control_id))
    }

    pub(super) fn apply(
        &mut self,
        session: &mut SessionHandle,
        model: &TouchOverlayModel,
        device_id: i32,
        key: AbstractKey,
        pressed: bool,
    ) {
        let changed = if pressed {
            self.pressed.insert((device_id, key))
        } else {
            self.pressed.remove(&(device_id, key))
        };
        if !changed {
            return;
        }
        let effective_pressed = self
            .pressed
            .iter()
            .any(|(_, pressed_key)| *pressed_key == key);
        let Some((attachment, control)) = Self::control_for_key(model, key) else {
            return;
        };
        let event = if effective_pressed {
            nerust_input_traits::DigitalInputEvent::pressed(attachment, control)
        } else {
            nerust_input_traits::DigitalInputEvent::released(attachment, control)
        };
        session.apply_input_event(event);
    }
}
