//! Standalone native message dialogs for boot-failure and crash paths.
//!
//! The framework owns only the vocabulary: severity, buttons, and text.
//! Each flavor exposes `show_message_dialog` with native blocking semantics,
//! safe to call with no event loop running (boot failure) or after the loop
//! died (crash containment). Callers always log to stderr as well, so a
//! missing display degrades to the log, never silence.

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
    /// Copy stays inline and English; it must not depend on the settings
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
        let dialog = MessageDialog::error("nerust", "boom");
        assert_eq!(dialog.title, "nerust");
        assert_eq!(dialog.description, "boom");
        assert_eq!(dialog.level, MessageLevel::Error);
        assert_eq!(dialog.buttons, MessageButtons::Ok);
    }
}
