/// Object-safe window-system surface behind every aux handle.
/// Lets the focus/registry layers route without naming the program type.
pub(crate) trait AuxShellOps {
    fn native_id(&self) -> crate::native_input::NativeWindowId;
    fn close_requested(&self) -> bool;
    fn focus_window(&self);
    fn request_redraw(&self);
    fn handle_common(&mut self, event: &crate::native_input::AuxNativeEvent);
    fn feed_event(&mut self, event: crate::native_input::AuxNativeEvent) -> Option<iced::Event>;
    fn render_tagged(&mut self, tag: &str) -> Option<String>;
    fn wake_deadline(&self) -> Option<std::time::Instant> {
        None
    }
    fn wake_due(&mut self, _now: std::time::Instant) {}
}

mod presenter;
mod pump;
mod surface;

pub(crate) use presenter::AuxShell;
pub(crate) use surface::AuxSurface;
