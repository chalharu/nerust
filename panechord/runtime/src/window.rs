use crate::{Session, UiHost, WindowHooks};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyCode(String);

impl KeyCode {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
