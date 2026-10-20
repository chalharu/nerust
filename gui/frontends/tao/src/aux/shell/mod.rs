/// Object-safe window-system surface behind every aux handle.
/// Lets the focus/registry layers route without naming the program type.
pub(crate) trait AuxShellOps {
    fn native_id(&self) -> crate::aux::domain::NativeWindowId;
    fn close_requested(&self) -> bool;
    fn focus_window(&self);
    fn request_redraw(&self);
    fn handle_common(&mut self, event: &crate::aux::domain::AuxNativeEvent);
    fn feed_event(&mut self, event: crate::aux::domain::AuxNativeEvent) -> Option<iced::Event>;
    fn render_tagged(&mut self, tag: &'static str);
}

mod presenter;
mod pump;
mod surface;

pub(crate) use presenter::AuxShell;
#[cfg(test)]
pub(crate) use pump::UiShell;
pub(crate) use surface::{AuxSurface, aux_settings, aux_theme};
