//! Re-exported from the `wakui_gtk` platform backend. Policy and
//! window lifecycle stay native; interpretation lives in the backend.

pub(crate) use wakui_gtk::controls::*;

#[cfg(test)]
use gtk::{glib, prelude::*};
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
mod tests;
