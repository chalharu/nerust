use crate::{KeyCode, Session, UiHost, WindowHooks};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[derive(Clone, Debug)]
pub enum WindowEvent {
    Focused(bool),
    KeyboardInput {
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        synthetic: bool,
    },
}

pub trait WindowSurface: HasDisplayHandle + HasWindowHandle {
    fn scale_factor(&self) -> f64;
    fn toggle_fullscreen(&self);
}

pub trait WindowHost<V>: UiHost {
    fn register_view(
        &self,
        session: &Session,
        id: &str,
        view: &V,
        hooks: WindowHooks,
    ) -> Result<(), String>;
}

/// Backend-resolved run configuration, injected at startup. Views consume
/// plain data instead of querying backend capabilities, so every branch stays
/// unit-testable on any platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnerConfig {
    /// Whether the menubar needs an explicit leading application menu: AppKit
    /// consumes the first installed submenu as the application menu, so
    /// without one the first application menu is swallowed.
    pub application_menu: bool,
    /// Whether Quit lives in the File menu. Menubar hosts set this; a future
    /// headerbar-style host clears it and excludes `MenuRole::Quit` leaves at
    /// render time instead, so YAML never forks per platform.
    pub menu_quit_in_file: bool,
}
