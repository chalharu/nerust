//! Window helpers over the generic framework windows. Identity and contents
//! stay framework-owned; titles and kind mapping are view definitions.

use wakui::chrome::window::Window;

use crate::focus::AuxKind;
use wakui::chrome::aux::AuxId;

/// Pending (pre-screen) aux window for a view-side kind.
pub fn pending_window(id: AuxId, kind: AuxKind) -> Window {
    Window::auxiliary(id, kind.title())
}
