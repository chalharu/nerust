//! Session presentation sync: menu, title, and fullscreen state.
use super::HostState;
use super::chrome::MenuSig;
use super::derive_native_fullscreen_sync;
use super::frame::FrameCtx;
use nerust_gui_shell::session::osd::TickMetrics;
use nerust_gui_shell::session::{SessionError, access::SettingsResult};
use std::time::Instant;

impl HostState {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(is_fullscreen) = self.windows.with_usable(|w| w.fullscreen().is_some()) else {
            return;
        };
        let sync = derive_native_fullscreen_sync(
            self.windows.pending_fullscreen_sync(),
            is_fullscreen,
            self.session
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        );
        self.windows
            .set_pending_fullscreen_sync(sync.pending_target);
        if !sync.persist_setting {
            return;
        }
        match self.session.set_fullscreen_default(is_fullscreen) {
            Ok(plan) if plan.fullscreen_default_changed => {
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, true);
                self.refresh_window_title(&ctx.tick());
            }
            Ok(_) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    /// Menu sync using scalar state and immutable snapshot identities only.
    pub(crate) fn sync_menu_state(&mut self, ctx: &FrameCtx) {
        use super::chrome::ChromeScalars;
        let settings_open = self.settings_open();
        let language = ctx.settings().shared.general.language;
        let active_slot = self.session.active_slot_id();
        let scalars = ChromeScalars {
            loaded: ctx.tick().loaded,
            paused: ctx.tick().paused,
            settings_open,
            language,
            active_slot,
        };
        if self.chrome.cheap_same(&scalars) && self.chrome.menu_same(ctx.slots(), ctx.shortcuts()) {
            return;
        }
        let sig = MenuSig {
            loaded: scalars.loaded,
            paused: scalars.paused,
            settings_open,
            language,
            active_slot,
            slots: std::sync::Arc::clone(ctx.slots()),
            shortcuts: std::sync::Arc::clone(ctx.shortcuts()),
        };
        self.app_menu.update(crate::menu::MenuSync {
            loaded: sig.loaded,
            paused: sig.paused,
            slots: &sig.slots,
            active_slot: sig.active_slot,
            settings_open: sig.settings_open,
            language: sig.language,
            shortcuts: &sig.shortcuts,
        });
        self.chrome.push_menu(sig);
    }

    fn push_title(&mut self, title: &str) {
        if let Some(window) = self.windows.live() {
            window.set_title(title);
        }
    }

    /// Title-only gate over the cached session title: one format per change.
    pub(crate) fn refresh_window_title(&mut self, tick: &TickMetrics) {
        if self
            .chrome
            .title_same(self.session.window_title_cached(tick))
        {
            return;
        }
        let title = self.session.window_title_cached(tick).to_owned();
        self.chrome.push_title(title.clone());
        self.push_title(&title);
    }

    pub(super) fn maybe_refresh_window_title(&mut self, ctx: &FrameCtx, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title(&ctx.tick());
        }
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        let Some(fullscreen) = self.windows.live().map(|w| w.fullscreen().is_none()) else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.windows.live() {
                window.set_fullscreen(
                    fullscreen.then_some(tao::window::Fullscreen::Borderless(None)),
                );
            }
            self.request_redraw();
        }
    }

    /// Refresh chrome after a ROM (re)load: menu, title, frame.
    pub(super) fn after_rom_load(&mut self) {
        self.reconcile_pause();
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
        if self.is_debug_open() {
            self.run_debug_drain(&[nerust_gui_shell::debug::DebugAction::Refresh]);
        }
    }

    /// Single chrome-sync choke point: menu, title, and optionally a frame.
    pub(crate) fn sync_chrome(&mut self, ctx: &FrameCtx, redraw: bool) {
        self.sync_menu_state(ctx);
        self.refresh_window_title(&ctx.tick());
        if redraw {
            self.request_redraw();
        }
    }

    /// Chrome pump for the control-flow pass: native state plus throttled title.
    pub(super) fn tick_chrome_sync(&mut self, ctx: &FrameCtx) {
        self.sync_fullscreen_default_from_window();
        self.maybe_refresh_window_title(ctx, Instant::now());
    }

    pub(super) fn sync_fullscreen_from_settings(&mut self) {
        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let is_fullscreen = self
            .windows
            .live()
            .is_some_and(|window| window.fullscreen().is_some());
        if is_fullscreen == fullscreen {
            self.windows.set_pending_fullscreen_sync(None);
            return;
        }
        self.windows.set_pending_fullscreen_sync(Some(fullscreen));
        if let Some(window) = self.windows.live() {
            window.set_fullscreen(fullscreen.then_some(tao::window::Fullscreen::Borderless(None)));
        }
    }

    pub(super) fn persist_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
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
        let title = host
            .session_mut()
            .window_title_cached(&ctx.tick())
            .to_owned();
        assert!(!host.chrome.title_same(&title));
        host.refresh_window_title(&ctx.tick());
        assert!(host.chrome.title_same(&title));
        host.refresh_window_title(&ctx.tick());
        assert!(host.chrome.title_same(&title));
    }

    #[test]
    fn chrome_choke_point_syncs_menu_and_title() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        host.sync_chrome(&ctx, true);
        assert_eq!(support.menu.updates().len(), 1);
        let title = host
            .session_mut()
            .window_title_cached(&ctx.tick())
            .to_owned();
        assert!(host.chrome.title_same(&title));
    }

    #[test]
    fn fullscreen_helpers_stay_total_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        host.sync_fullscreen_from_settings();
        assert!(host.windows.pending_fullscreen_sync().is_none());
        host.sync_fullscreen_default_from_window();
        host.toggle_fullscreen();
        assert!(host.windows.pending_fullscreen_sync().is_none());
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
        host.maybe_refresh_window_title(&ctx, now);
        let title = host
            .session_mut()
            .window_title_cached(&ctx.tick())
            .to_owned();
        assert!(!host.chrome.title_same(&title));
    }
}
