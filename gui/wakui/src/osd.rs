//! Framework OSD ports: modal presentation and turn outcomes.
//! Raster types stay view-side; only plain presentation crosses here.

use std::sync::mpsc::Receiver;

use crate::loop_driver::WakeNeed;

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

/// Owned pump outcome: optional modal, wake needs, and a changed flag.
#[derive(Debug, Default)]
pub struct OsdTurn {
    pub modal: Option<ModalPresentation>,
    pub wake: WakeNeed,
    pub changed: bool,
}
