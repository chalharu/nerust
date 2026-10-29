//! Native dialogs: retained lifecycle, modal presentation, boot messages.
//! The host owns the modal queue; this only finishes exactly once on close
//! signals, presents queued modals, and blocks for boot-failure paths.
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::mpsc::Receiver,
};

pub type FinishCallback = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// Native close signals may nest. Take the callback before running any effect.
pub fn finish(callback: &FinishCallback) {
    let finish = callback.borrow_mut().take();
    if let Some(finish) = finish {
        finish();
    }
}

pub fn connect_finish(dialog: &gtk::Dialog, callback: &FinishCallback) {
    let on_destroy = callback.clone();
    dialog.connect_destroy(move |_| finish(&on_destroy));
    let on_unrealize = callback.clone();
    dialog.connect_unrealize(move |_| finish(&on_unrealize));
    let on_close = callback.clone();
    dialog.connect_close_request(move |_| {
        finish(&on_close);
        glib::Propagation::Proceed
    });
}

use wakui_core::chrome::aux::AuxId;
use wakui_core::chrome::dialog::{ModalAdvance, ModalPresentation, ModalPresenter};
use wakui_core::chrome::mailbox::WindowMailbox;
use wakui_core::chrome::window::Window as HostWindow;
use wakui_core::scene::control::ControlScreen;

use crate::State;
use crate::chrome::surface::{Surface, SurfaceCore};
use crate::chrome::window::{GtkWindowRenderer, Window, WindowExtend};
use crate::scene::binding::{Callbacks, WidgetDialog};

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

use gtk::{glib, prelude::*};

/// Show a native acknowledgment dialog and block until it is dismissed.
/// Returns immediately when no display is available.
pub fn show_message_dialog(dialog: &wakui_core::chrome::dialog::MessageDialog) {
    if !gtk::is_initialized() && gtk::init().is_err() {
        return;
    }
    let native = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        match dialog.level {
            wakui_core::chrome::dialog::MessageLevel::Info => gtk::MessageType::Info,
            wakui_core::chrome::dialog::MessageLevel::Warning => gtk::MessageType::Warning,
            wakui_core::chrome::dialog::MessageLevel::Error => gtk::MessageType::Error,
        },
        match dialog.buttons {
            wakui_core::chrome::dialog::MessageButtons::Ok => gtk::ButtonsType::Ok,
        },
        &dialog.description,
    );
    native.set_title(Some(dialog.title.as_str()));
    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    native.connect_response(move |dialog, _| {
        dialog.destroy();
        quit.quit();
    });
    native.present();
    main_loop.run();
}

pub(crate) struct DialogRenderer<'a> {
    pub(crate) parent: &'a gtk::ApplicationWindow,
    pub(crate) screen: Option<ControlScreen>,
    pub(crate) mailbox: Option<WindowMailbox>,
    pub(crate) callbacks: Option<Callbacks>,
    pub(crate) model: Option<HostWindow>,
    pub(crate) result: Option<RenderedDialog>,
}

pub(crate) enum RenderedDialog {
    Controls(WidgetDialog),
}

pub(crate) fn render_dialog(
    model: &HostWindow,
    parent: &gtk::ApplicationWindow,
    _id: AuxId,
    screen: ControlScreen,
    mailbox: WindowMailbox,
    callbacks: Callbacks,
) -> Option<RenderedDialog> {
    // `Build` never touches the `Main` variant, so any aux kind works.
    let mut renderer = GtkWindowRenderer::<()>::Build(Box::new(DialogRenderer {
        parent,
        screen: Some(screen),
        mailbox: Some(mailbox),
        callbacks: Some(callbacks),
        model: None,
        result: None,
    }));
    model.visit(&mut renderer);
    let GtkWindowRenderer::Build(builder) = renderer else {
        unreachable!()
    };
    builder.result
}

pub(crate) fn open_about<K: Copy + Eq + std::hash::Hash + 'static>(window: &Window<K>) {
    let existing = window.borrow().about_dialog.clone();
    if let Some(dialog) = existing {
        dialog.present();
        return;
    }
    {
        let mut core = window.borrow_mut();
        if core.about_opening {
            return;
        }
        core.about_opening = true;
    }
    let dialog: gtk::AboutDialog =
        gtk::Builder::from_string(include_str!("../../resources/about.xml"))
            .object("about")
            .unwrap();
    dialog.set_transient_for(Some(&window.window()));
    let weak = Rc::downgrade(window);
    dialog.connect_close_request(move |dialog| {
        if let Some(window) = weak.upgrade() {
            let old = {
                let mut core = window.borrow_mut();
                if core.about_dialog.as_ref() == Some(dialog) {
                    core.about_dialog.take()
                } else {
                    None
                }
            };
            drop(old);
        }
        glib::Propagation::Proceed
    });
    {
        let mut core = window.borrow_mut();
        core.about_dialog = Some(dialog.clone());
        core.about_opening = false;
    }
    dialog.present();
}

pub(crate) fn show_file_load_error<K: Copy + Eq + std::hash::Hash + 'static>(
    _parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State<K>>>,
    detail: &str,
) {
    state.borrow_mut().host.report_host_error(detail.to_owned());
}
