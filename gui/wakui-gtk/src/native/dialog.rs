use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::mpsc::Receiver,
};

use gtk::{glib, prelude::*};
use wakui::osd::{ModalAdvance, ModalPresentation, ModalPresenter};

use crate::surface::{Surface, SurfaceCore};

/// Only translates the host token into a native response. Host retains the
/// queue, pause ownership and stale-ACK checks; there is no native ModalState.
pub(crate) fn present_modal(
    surface: &Surface,
    window: &gtk::ApplicationWindow,
    presentation: ModalPresentation,
) {
    let port = GtkModalPresenter {
        surface: Rc::downgrade(surface),
        window: window.downgrade(),
    };
    let presenter: &dyn ModalPresenter = &port;
    let token = presentation.token;
    if let Some(receiver) = presenter.present_modal(presentation) {
        let state = surface.borrow().state.clone();
        state
            .borrow_mut()
            .host
            .attach_osd_modal_receiver(token, receiver);
    }
}

struct GtkModalPresenter {
    surface: Weak<RefCell<SurfaceCore>>,
    window: glib::WeakRef<gtk::ApplicationWindow>,
}

impl ModalPresenter for GtkModalPresenter {
    fn present_modal(&self, presentation: ModalPresentation) -> Option<Receiver<()>> {
        let window = self.window.upgrade()?;
        let dialog = gtk::MessageDialog::new(
            Some(&window),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Ok,
            &presentation.detail,
        );
        let surface = self.surface.clone();
        let window = self.window.clone();
        dialog.connect_response(move |dialog, _| {
            // close() can emit DeleteEvent again and ACK twice.
            dialog.destroy();
            let Some(surface) = surface.upgrade() else {
                return;
            };
            let state = surface.borrow().state.clone();
            let next = state
                .borrow_mut()
                .host
                .acknowledge_osd_modal(presentation.token);
            match next {
                ModalAdvance::Present(next) => {
                    // Bounded by the host queue, even with synchronous map ACK.
                    if let Some(window) = window.upgrade() {
                        present_modal(&surface, &window, next);
                    }
                }
                ModalAdvance::Ignored | ModalAdvance::Drained => {}
            }
        });
        dialog.present();
        None
    }
}
