//! Session presentation sync: menu, title, and fullscreen state.
use super::{ChromeScalars, FrameCtx, HostState, MenuSig};
use super::{derive_native_fullscreen_sync, set_window_fullscreen, window_is_fullscreen};
use nerust_gui_shell::session::osd::TickMetrics;
use nerust_gui_shell::session::{SessionError, access::SettingsResult};
use std::time::Instant;

impl HostState {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(window) = self.windows.live() else {
            return;
        };
        let fullscreen = window_is_fullscreen(window.raw());
        let sync = derive_native_fullscreen_sync(
            self.windows.pending_fullscreen_sync(),
            fullscreen,
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
        match self.session.set_fullscreen_default(fullscreen) {
            Ok(plan) if plan.fullscreen_default_changed => {
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, true);
                self.refresh_window_title(&ctx.tick);
            }
            Ok(_) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    /// Menu sync against one pass capture: scalars, then one
    /// fingerprint, then the exact slice compare. No per-call clones.
    pub(crate) fn sync_menu_state(&mut self, ctx: &FrameCtx) {
        let settings_open = self.settings_open();
        let language = ctx.settings.shared.general.language;
        let active_slot = self.session.active_slot_id();
        let scalars = ChromeScalars {
            loaded: ctx.tick.loaded,
            paused: ctx.tick.paused,
            settings_open,
            language,
            active_slot,
            slots_len: ctx.slots.len(),
            shortcuts_len: ctx.shortcuts.len(),
        };
        // Computed once per call: the cheap gate usually skips the IPC below.
        let generation = MenuSig::fingerprint(&ctx.slots, &ctx.shortcuts);
        if self.chrome.cheap_same(&scalars)
            && self
                .chrome
                .menu_same(generation, &ctx.slots, &ctx.shortcuts)
        {
            return;
        }
        let sig = MenuSig {
            loaded: scalars.loaded,
            paused: scalars.paused,
            settings_open,
            language,
            active_slot,
            slots: std::sync::Arc::clone(&ctx.slots),
            shortcuts: std::sync::Arc::clone(&ctx.shortcuts),
            generation,
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
        self.refresh_window_title(&ctx.tick);
    }

    fn push_title(&mut self, title: &str) {
        if let Some(window) = self.windows.live() {
            window.set_title(title);
        }
    }

    /// Title-only gate over the cached session title: no formatting on a repeat.
    pub(crate) fn refresh_window_title(&mut self, tick: &TickMetrics) {
        let changed = {
            let title = self.session.window_title_cached(tick);
            !self.chrome.title_same(title)
        };
        if changed {
            let title = self.session.window_title_cached(tick).to_owned();
            self.chrome.push_title(title.clone());
            self.push_title(&title);
        }
    }

    pub(super) fn maybe_refresh_window_title(&mut self, ctx: &FrameCtx, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title(&ctx.tick);
        }
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        let fullscreen = if let Some(window) = self.windows.live() {
            !window_is_fullscreen(window.raw())
        } else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.windows.live() {
                set_window_fullscreen(window.raw(), fullscreen);
            }
            self.request_redraw();
        }
    }

    /// Refresh chrome after a ROM (re)load: menu, title, frame.
    pub(super) fn after_rom_load(&mut self, ctx: &FrameCtx) {
        self.sync_chrome(ctx, true);
    }

    /// Single chrome-sync choke point: menu, title, and optionally a frame.
    pub(crate) fn sync_chrome(&mut self, ctx: &FrameCtx, redraw: bool) {
        self.sync_menu_state(ctx);
        self.refresh_window_title(&ctx.tick);
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
            .is_some_and(|window| window_is_fullscreen(window.raw()));
        if is_fullscreen == fullscreen {
            self.windows.set_pending_fullscreen_sync(None);
            return;
        }
        self.windows.set_pending_fullscreen_sync(Some(fullscreen));
        if let Some(window) = self.windows.live() {
            set_window_fullscreen(window.raw(), fullscreen);
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
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}
