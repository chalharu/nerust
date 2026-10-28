//! Session presentation sync: menu, title, and fullscreen state.
use super::HostState;
use crate::menu::TAO_MENU_CAPABILITIES;
use nerust_frontend_host::frame::FrameCtx;
use std::time::Instant;

use nerust_frontend_host::{
    application::{MenuSig, MenubarUpdate},
    menu::Menubar,
    window::{Window, WindowVisitor},
};

struct MainPresentation<'a> {
    native: Option<&'a tao::window::Window>,
    menu: Option<(&'a mut dyn crate::menu::MenuSink, MenuSig)>,
}

impl WindowVisitor for MainPresentation<'_> {
    fn begin_window(&mut self, window: &Window) {
        if let Some(native) = self.native {
            native.set_title(&window.title);
        }
    }

    fn visit_menu(&mut self, menu: &Menubar) {
        if let Some((sink, signature)) = self.menu.as_mut() {
            sink.update(MenubarUpdate {
                signature: signature.clone(),
                menubar: menu.clone(),
            });
        }
    }
}

impl HostState {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(is_fullscreen) = self.windows.with_usable(|w| w.fullscreen().is_some()) else {
            return;
        };
        if !self.core.observe_fullscreen(is_fullscreen) {
            return;
        }
        match self.core.persist_fullscreen_default(is_fullscreen) {
            Ok(true) => {
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, true);
                self.refresh_window_title();
            }
            Ok(false) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    /// Menu sync using scalar state and immutable snapshot identities only.
    pub(crate) fn sync_menu_state(&mut self, ctx: &FrameCtx) {
        let settings_open = self.settings_open();
        let Some(update) = self
            .core
            .menubar_update(ctx, settings_open, TAO_MENU_CAPABILITIES)
        else {
            return;
        };
        let window = self.core.main_window().clone();
        window.visit(&mut MainPresentation {
            native: None,
            menu: Some((self.app_menu.as_mut(), update.signature)),
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

    pub(super) fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.core.title_refresh_due(now) {
            self.refresh_window_title();
        }
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        let Some(fullscreen) = self.windows.live().map(|w| w.fullscreen().is_none()) else {
            return;
        };
        match self.core.persist_fullscreen_default(fullscreen) {
            Ok(_) => {
                self.sync_fullscreen_from_settings();
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, true);
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

    /// Refresh chrome after a ROM (re)load: menu, title, frame.
    pub(super) fn after_rom_load(&mut self) {
        self.reconcile_pause();
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
        if self.is_debug_open() {
            if let Some(bridge) = self.debug_bridge_for_ppu() {
                use crate::aux::debug::DebugMessage;
                bridge.dispatch(DebugMessage::RefreshPressed);
            }
            self.drain_debug_requests();
        }
    }

    /// Single chrome-sync choke point: menu, title, and optionally a frame.
    pub(crate) fn sync_chrome(&mut self, ctx: &FrameCtx, redraw: bool) {
        self.sync_menu_state(ctx);
        self.refresh_window_title();
        if redraw {
            self.request_redraw();
        }
    }

    /// Chrome pump for the control-flow pass: native state plus throttled title.
    pub(super) fn tick_chrome_sync(&mut self) {
        self.sync_fullscreen_default_from_window();
        self.maybe_refresh_window_title(Instant::now());
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
        let ctx = host.frame_ctx();
        host.sync_menu_state(&ctx);
        assert_eq!(support.menu.updates().len(), 1);
        // Same capture: scalar state and Arc identities both hold.
        let ctx = host.frame_ctx();
        host.sync_menu_state(&ctx);
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn title_gate_pushes_once_per_change() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        let title = host.cached_title(&ctx.tick());
        assert!(!host.title_same(&title));
        host.refresh_window_title();
        assert!(host.title_same(&title));
        host.refresh_window_title();
        assert!(host.title_same(&title));
    }

    #[test]
    fn chrome_choke_point_syncs_menu_and_title() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        host.sync_chrome(&ctx, true);
        assert_eq!(support.menu.updates().len(), 1);
        let title = host.cached_title(&ctx.tick());
        assert!(host.title_same(&title));
    }

    #[test]
    fn fullscreen_helpers_stay_total_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        host.sync_fullscreen_from_settings();
        assert!(host.pending_fullscreen().is_none());
        host.sync_fullscreen_default_from_window();
        host.toggle_fullscreen();
        assert!(host.pending_fullscreen().is_none());
    }

    #[test]
    fn rom_load_refresh_routes_through_chrome() {
        let support = TestHost::new();
        let mut host = support.host;
        host.after_rom_load();
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn title_throttle_blocks_back_to_back_refresh() {
        let support = TestHost::new();
        let mut host = support.host;
        let now = std::time::Instant::now();
        let ctx = host.frame_ctx();
        // Fresh shell just stamped the title clock, so an immediate
        // refresh stays gated until the interval elapses.
        host.maybe_refresh_window_title(now);
        let title = host.cached_title(&ctx.tick());
        assert!(!host.title_same(&title));
    }
}
