//! Retained dialog lifecycle: finish exactly once on close signals.
//!
//! Native close signals may nest; the callback is taken before running any
//! effect. Modal presentation stays frontend-side (it owns the app turn).

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

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
