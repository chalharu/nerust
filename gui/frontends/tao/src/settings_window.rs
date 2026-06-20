#![allow(dead_code)]

use iced_tiny_skia::window::{Compositor, Surface};
use iced_tiny_skia::Renderer;

pub(crate) struct SettingsWindowHandle;

pub(crate) struct SettingsRenderer {
    pub(crate) compositor: Compositor,
    pub(crate) surface: Surface,
    pub(crate) renderer: Renderer,
}
