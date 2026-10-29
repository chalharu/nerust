//! Re-exported from the `wakui_tao` platform backend. Policy and
//! window lifecycle stay native; interpretation lives in the backend.

#[cfg_attr(not(test), allow(unused_imports))]
pub(crate) use crate::controls::*;

#[cfg(test)]
use crate::binding::{El, Message};
#[cfg(test)]
use wakui_core::control::{ControlDisplay, ControlInput, ControlMetadata, ControlScreen, Page};

#[cfg(test)]
mod tests;

#[cfg(test)]
#[derive(Default)]
struct ControlsWindowVisitor {
    output: Option<El<'static>>,
}

#[cfg(test)]
impl wakui_core::window::WindowVisitor for ControlsWindowVisitor {
    fn visit_controls(&mut self, screen: &wakui_core::control::ControlScreen) {
        self.output = Some(build_screen(screen));
    }
}

#[cfg(test)]
pub(crate) fn build(window: &wakui_core::window::Window) -> El<'static> {
    let mut visitor = ControlsWindowVisitor::default();
    window.visit(&mut visitor);
    visitor
        .output
        .expect("widget window must contain a control screen")
}
