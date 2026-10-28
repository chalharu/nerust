//! Toolkit-free menu identity and title history gates.

use crate::frame::FrameCtx;
use std::sync::Arc;

use crate::menu::MenuSig;

#[derive(Default)]
pub(super) struct ChromeSync {
    last_menu: Option<MenuSig>,
    last_title: Option<String>,
}

impl ChromeSync {
    pub fn menu_update(
        &mut self,
        ctx: &FrameCtx,
        settings_open: bool,
        active_slot: Option<u64>,
    ) -> Option<MenuSig> {
        if self.last_menu.as_ref().is_some_and(|sig| {
            sig.loaded == ctx.tick().loaded
                && sig.paused == ctx.tick().paused
                && sig.settings_open == settings_open
                && sig.language == ctx.settings().shared.general.language
                && sig.active_slot == active_slot
                && Arc::ptr_eq(&sig.slots, ctx.slots())
                && Arc::ptr_eq(&sig.shortcuts, ctx.shortcuts())
        }) {
            return None;
        }
        let sig = MenuSig {
            loaded: ctx.tick().loaded,
            paused: ctx.tick().paused,
            settings_open,
            language: ctx.settings().shared.general.language,
            active_slot,
            slots: Arc::clone(ctx.slots()),
            shortcuts: Arc::clone(ctx.shortcuts()),
        };
        self.last_menu = Some(sig.clone());
        Some(sig)
    }

    pub fn menu_signature(&self) -> Option<&MenuSig> {
        self.last_menu.as_ref()
    }

    pub fn title_same(&self, title: &str) -> bool {
        self.last_title.as_deref() == Some(title)
    }
    pub fn title_update(&mut self, title: String) -> Option<String> {
        if self.title_same(&title) {
            return None;
        }
        self.last_title = Some(title.clone());
        Some(title)
    }
}
