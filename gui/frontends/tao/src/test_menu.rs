//! Headless menu sink: records `MenuSync` snapshots for assertions.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};

use crate::menu::{MenuSink, MenuSync};

/// `MenuSink` that records sync snapshots for assertions.
#[derive(Debug, Default)]
pub(crate) struct MenuRecorder {
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
        crate::aux::bridge::lock_recovered(&self.inner.updates).clone()
    }

    pub(crate) fn into_box(self) -> Box<dyn MenuSink> {
        Box::new(self)
    }
}

impl MenuSink for MenuRecorderHandle {
    fn update(&mut self, sync: MenuSync<'_>) {
        crate::aux::bridge::lock_recovered(&self.inner.updates).push(MenuSyncSnapshot {
            loaded: sync.loaded,
            paused: sync.paused,
            settings_open: sync.settings_open,
        });
    }

    fn init_for_window(&self, _window: &tao::window::Window) {}

    fn clear_event_handler(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuSyncSnapshot {
    pub(crate) loaded: bool,
    pub(crate) paused: bool,
    pub(crate) settings_open: bool,
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
