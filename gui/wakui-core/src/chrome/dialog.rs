//! Standalone native message dialogs for boot-failure and crash paths,
//! plus the generic modal queue vocabulary for runtime error dialogs.
//!
//! The framework owns only the vocabulary: severity, buttons, text, and the
//! owned modal effect (`ModalPresentation` + `ModalToken` + `ModalAdvance`).
//! View definitions own the queue, pause arbitration, and localized text;
//! each flavor presents the owned effect after the application borrow ends
//! through [`ModalPresenter`]. No toolkit types cross this boundary.
//!
//! Each flavor exposes `show_message_dialog` with native blocking semantics,
//! safe to call with no event loop running (boot failure) or after the loop
//! died (crash containment). Callers always log to stderr as well, so a
//! missing display degrades to the log, never silence.

use std::sync::mpsc::Receiver;

use crate::flow::loop_driver::WakeNeed;

/// Dialog severity. Flavors map this to their native message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Warning,
    Error,
}

/// Dialog buttons. Only acknowledgment exists today; extend when a caller
/// needs a real choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageButtons {
    Ok,
}

/// Owned dialog description. No toolkit types cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDialog {
    pub title: String,
    pub description: String,
    pub level: MessageLevel,
    pub buttons: MessageButtons,
}

impl MessageDialog {
    /// Error acknowledgment: the only shape boot/crash paths need.
    /// Copy stays inline and English; it must not depend on the caller
    /// stack that may have just died.
    pub fn error(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            level: MessageLevel::Error,
            buttons: MessageButtons::Ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_acknowledgment_carries_text_and_severity() {
        let dialog = MessageDialog::error("app", "boom");
        assert_eq!(dialog.title, "app");
        assert_eq!(dialog.description, "boom");
        assert_eq!(dialog.level, MessageLevel::Error);
        assert_eq!(dialog.buttons, MessageButtons::Ok);
    }
}

/// Native presentation runs only after the application borrow has ended.
/// Callback-based dialogs return None; worker dialogs return their ACK receiver.
pub trait ModalPresenter {
    fn present_modal(&self, presentation: ModalPresentation) -> Option<Receiver<()>>;
}

/// Owned native effect. Take it under the host borrow, present only after release.
#[derive(Debug, PartialEq, Eq)]
pub struct ModalPresentation {
    pub token: ModalToken,
    pub detail: String,
}

/// Process-unique dialog identity, including across runtime replacement/close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalToken(pub u64);

/// ACK outcome shared by threaded dialogs and native widget callbacks.
/// The app commits pause release before returning this owned effect.
#[derive(Debug, PartialEq, Eq)]
pub enum ModalAdvance {
    Ignored,
    Present(ModalPresentation),
    Drained,
}

/// Owned pump outcome: optional modal plus wake needs.
/// Transient notification layers live renderer-side; only the modal
/// (a generic dialog) crosses into native code.
#[derive(Debug, Default)]
pub struct DialogTurn {
    pub modal: Option<ModalPresentation>,
    pub wake: WakeNeed,
}
