//! Toolkit-free menu identity and title history gates.

use crate::frame::FrameCtx;
use std::sync::Arc;

/// Pre-localized dialog titles. Frontends render and pass these verbatim
/// instead of calling into the i18n catalog themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogTitles {
    pub open: String,
    pub error: String,
    pub rom_load_failed: String,
}

impl DialogTitles {
    pub fn new(language: nerust_settings_core::model::language::AppLanguage) -> Self {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        Self {
            open: ui_text(language, UiText::Open).to_string(),
            error: ui_text(language, UiText::Error).to_string(),
            rom_load_failed: ui_text(language, UiText::RomLoadFailed).to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MenuSig {
    pub loaded: bool,
    pub paused: bool,
    pub settings_open: bool,
    pub language: nerust_settings_core::model::language::AppLanguage,
    pub active_slot: Option<u64>,
    pub slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
    pub shortcuts: Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
}

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
