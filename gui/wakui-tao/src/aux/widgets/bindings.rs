//! Re-exported from the `wakui_tao` platform backend. Policy and
//! window lifecycle stay native; interpretation lives in the backend.

pub(crate) use crate::binding::*;

#[cfg(test)]
#[cfg(test)]
pub(crate) use crate::handler::WidgetBinding;
#[cfg(test)]
use std::rc::Rc;
#[cfg(test)]
use wakui::Key;

#[cfg(test)]
pub(crate) mod tests;
