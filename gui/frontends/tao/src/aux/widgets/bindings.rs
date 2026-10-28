//! Re-exported from the `wakui_tao` platform backend. Policy and
//! window lifecycle stay native; interpretation lives in the backend.

pub(crate) use wakui_tao::binding::*;

#[cfg(test)]
use std::rc::Rc;
#[cfg(test)]
use wakui::Key;
#[cfg(test)]
use wakui::focus::AuxKind;

#[cfg(test)]
pub(crate) use wakui_tao::handler::WidgetBinding;

#[cfg(test)]
pub(crate) mod tests;
