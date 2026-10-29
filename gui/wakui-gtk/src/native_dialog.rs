use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::mpsc::Receiver,
};

use gtk::{glib, prelude::*};
use wakui_core::chrome::dialog::{ModalAdvance, ModalPresentation, ModalPresenter};

use crate::surface::{Surface, SurfaceCore};

/// Only translates the host token into a native response. Host retains the
/// queue and stale-ACK checks; there is no native ModalState.
pub(crate) fn present_modal<K: Copy + Eq + std::hash::Hash + 'static>(
    surface: &Surface<K>,
    window: &gtk::ApplicationWindow,
    presentation: ModalPresentation,
) {
    let port = GtkModalPresenter {
        surface: Rc::downgrade(surface),
        window: window.downgrade(),
    };
    let presenter: GtkModalPresenter<K> = port;
    let token = presentation.token;
    if let Some(receiver) = ModalPresenter::present_modal(&presenter, presentation) {
        let state = surface.borrow().state.clone();
        state
            .borrow_mut()
            .host
            .attach_dialog_receiver(token, receiver);
    }
}

struct GtkModalPresenter<K: Copy + Eq + std::hash::Hash + 'static> {
    surface: Weak<RefCell<SurfaceCore<K>>>,
    window: glib::WeakRef<gtk::ApplicationWindow>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> ModalPresenter for GtkModalPresenter<K> {
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
                .acknowledge_dialog(presentation.token);
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
