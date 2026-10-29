//! Headless menu sink: records shared `MenuSig` snapshots for assertions.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};
use wakui_core::sync::lock_recovered;

use crate::menu::MenuSink;
use wakui_core::menu::Menubar;

/// `MenuSink` that records sync snapshots for assertions.
#[derive(Debug, Default)]
struct MenuRecorder {
    updates: Mutex<Vec<MenuSyncSnapshot>>,
}

/// Shared handle to a recorder: the owned copy lives behind the
/// `Box<dyn MenuSink>`, tests read through the clone.
#[derive(Debug, Clone, Default)]
pub(crate) struct MenuRecorderHandle {
    inner: Arc<MenuRecorder>,
}

impl MenuRecorderHandle {
    pub(crate) fn updates(&self) -> Vec<MenuSyncSnapshot> {
        lock_recovered(&self.inner.updates).clone()
    }

    pub(crate) fn into_box(self) -> Box<dyn MenuSink> {
        Box::new(self)
    }
}

impl MenuSink for MenuRecorderHandle {
    fn update(&mut self, loaded: bool, paused: bool, _menu: &Menubar) {
        lock_recovered(&self.inner.updates).push(MenuSyncSnapshot { loaded, paused });
    }

    fn init_for_window(&self, _window: &tao::window::Window) {}

    fn clear_event_handler(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuSyncSnapshot {
    pub(crate) loaded: bool,
    pub(crate) paused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_window_fns_stay_total() {
        let recorder = MenuRecorderHandle::default();
        recorder.clear_event_handler();
        assert!(recorder.updates().is_empty());
    }
}
