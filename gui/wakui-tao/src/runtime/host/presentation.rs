//! Session presentation sync: menu, title, and fullscreen state.
use super::HostState;
use crate::menu::TAO_MENU_CAPABILITIES;

use wakui_core::{
    chrome::menu::Menubar,
    chrome::window::{Window, WindowVisitor},
};

struct MainPresentation<'a> {
    native: Option<&'a tao::window::Window>,
    menu: Option<(&'a mut dyn crate::menu::MenuSink, bool, bool)>,
}

impl WindowVisitor for MainPresentation<'_> {
    fn begin_window(&mut self, window: &Window) {
        if let Some(native) = self.native {
            native.set_title(&window.title);
        }
    }

    fn visit_menu(&mut self, menu: &Menubar) {
        if let Some((sink, loaded, paused)) = self.menu.as_mut() {
            sink.update(*loaded, *paused, menu);
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash + 'static> HostState<K> {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(is_fullscreen) = self.windows.with_usable(|w| w.fullscreen().is_some()) else {
            return;
        };
        if !self.core.observe_fullscreen(is_fullscreen) {
            return;
        }
        match self.core.persist_fullscreen_default(is_fullscreen) {
            Ok(true) => {
                self.sync_chrome(true);
                self.refresh_window_title();
            }
            Ok(false) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    /// Menu sync using scalar state and immutable snapshot identities only.
    pub(crate) fn sync_menu_state(&mut self) {
        // The modal guard is view-side state; the model already carries
        // guarded enablement, so flavors never name the guard kind.
        let loaded = self.core.content_loaded();
        let paused = self.core.content_suspended();
        let Some(_menubar) = self.core.menubar_update(TAO_MENU_CAPABILITIES) else {
            return;
        };
        let window = self.core.main_window().clone();
        window.visit(&mut MainPresentation {
            native: None,
            menu: Some((self.app_menu.as_mut(), loaded, paused)),
        });
    }

    /// Title-only gate over the cached session title: one format per change.
    pub(crate) fn refresh_window_title(&mut self) {
        if self.core.refresh_window_title().is_some() {
            let window = self.core.main_window().clone();
            window.visit(&mut MainPresentation {
                native: self.windows.live().map(AsRef::as_ref),
                menu: None,
            });
        }
    }

    /// Chrome pump for the control-flow pass: native state plus title.
    /// Title pulls are change-gated view-side, so every pass sees the
    /// latest title immediately without any timer.
    pub(super) fn tick_chrome_sync(&mut self) {
        self.sync_fullscreen_default_from_window();
        self.refresh_window_title();
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        let Some(fullscreen) = self.windows.live().map(|w| w.fullscreen().is_none()) else {
            return;
        };
        match self.core.persist_fullscreen_default(fullscreen) {
            Ok(_) => {
                self.sync_fullscreen_from_settings();
                self.sync_chrome(true);
            }
            Err(error) => {
                log::warn!("failed to persist fullscreen setting: {error}");
                if let Some(window) = self.windows.live() {
                    window.set_fullscreen(
                        fullscreen.then_some(tao::window::Fullscreen::Borderless(None)),
                    );
                }
                self.request_redraw();
            }
        }
    }

    /// Refresh chrome after a file (re)load: menu, title, frame.
    /// Suspension holds are reasserted view-side inside `load_file`;
    /// flavors only refresh chrome here.
    pub(super) fn after_file_load(&mut self) {
        self.sync_chrome(true);
        if self.core.polled_widget().is_some() {
            self.refresh_open_polled();
        }
    }

    /// Single chrome-sync choke point: menu, title, and optionally a frame.
    pub(crate) fn sync_chrome(&mut self, redraw: bool) {
        self.sync_menu_state();
        self.refresh_window_title();
        if redraw {
            self.request_redraw();
        }
    }

    pub(super) fn sync_fullscreen_from_settings(&mut self) {
        let is_fullscreen = self
            .windows
            .live()
            .is_some_and(|window| window.fullscreen().is_some());
        let Some(fullscreen) = self.core.fullscreen_request(is_fullscreen) else {
            return;
        };
        if let Some(window) = self.windows.live() {
            window.set_fullscreen(fullscreen.then_some(tao::window::Fullscreen::Borderless(None)));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    #[test]
    fn menu_sync_pushes_once_per_content() {
        let support = TestHost::new();
        let mut host = support.host;
        host.sync_menu_state();
        assert_eq!(support.menu.updates().len(), 1);
        // Same capture: scalar state and Arc identities both hold.
        host.sync_menu_state();
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn title_gate_pushes_once_per_change() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.core.refresh_window_title().is_some());
        assert!(host.core.refresh_window_title().is_none());
        assert!(host.core.refresh_window_title().is_none());
    }

    #[test]
    fn chrome_choke_point_syncs_menu_and_title() {
        let support = TestHost::new();
        let mut host = support.host;
        host.sync_chrome(true);
        assert_eq!(support.menu.updates().len(), 1);
        assert!(host.core.refresh_window_title().is_none());
    }

    #[test]
    fn fullscreen_helpers_stay_total_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        host.sync_fullscreen_from_settings();
        host.sync_fullscreen_default_from_window();
        host.toggle_fullscreen();
    }

    #[test]
    fn rom_load_refresh_routes_through_chrome() {
        let support = TestHost::new();
        let mut host = support.host;
        host.after_file_load();
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn title_refresh_publishes_once_then_stays_quiet_until_change() {
        let support = TestHost::new();
        let mut host = support.host;
        // Change-gated, not time-gated: the first pull publishes, repeats
        // stay quiet until the underlying state changes.
        host.refresh_title_after_present();
        assert!(host.core.refresh_window_title().is_none());
        host.load_test_file();
        assert!(host.core.refresh_window_title().is_some());
        assert!(host.core.refresh_window_title().is_none());
    }
}
