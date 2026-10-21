//! Chrome sync: menu signature plus window title, each with its own gate.
//! Owned by `HostState`; title-only moves never touch the menu path.

use std::sync::Arc;

/// Menu signature: scalars plus immutable source allocation identities.
#[derive(Debug, Clone)]
pub(super) struct MenuSig {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
    pub(super) slots: std::sync::Arc<[nerust_persistence::model::StateSlotSummary]>,
    pub(super) shortcuts: std::sync::Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
}

/// Scalar snapshot for the cheap chrome gate: title is gated separately.
pub(super) struct ChromeScalars {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
}

/// Chrome sync state: last pushed menu signature plus last pushed window title.
#[derive(Debug, Default)]
pub(super) struct ChromeSync {
    last_menu_sig: Option<MenuSig>,
    last_title: crate::util::Versioned<String>,
}

impl ChromeSync {
    pub(super) fn new() -> Self {
        Self {
            last_menu_sig: None,
            last_title: crate::util::Versioned::new(),
        }
    }

    /// Scalar gate; source identities are checked separately without scanning slices.
    pub(super) fn cheap_same(&self, scalars: &ChromeScalars) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            sig.loaded == scalars.loaded
                && sig.paused == scalars.paused
                && sig.settings_open == scalars.settings_open
                && sig.language == scalars.language
                && sig.active_slot == scalars.active_slot
        })
    }

    /// O(1) gate over immutable snapshots, retaining the previous allocations to avoid ABA.
    pub(super) fn menu_same(
        &self,
        slots: &Arc<[nerust_persistence::model::StateSlotSummary]>,
        shortcuts: &Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    ) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            Arc::ptr_eq(slots, &sig.slots) && Arc::ptr_eq(shortcuts, &sig.shortcuts)
        })
    }

    pub(super) fn push_menu(&mut self, sig: MenuSig) {
        self.last_menu_sig = Some(sig);
    }

    /// Title-only gate: never touches the menu signature.
    pub(super) fn title_same(&self, title: &str) -> bool {
        self.last_title.get().is_some_and(|last| last == title)
    }

    pub(super) fn push_title(&mut self, title: String) {
        self.last_title.check(&title);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;

    #[test]
    fn chrome_gates_skip_repeat_pushes() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        let scalars = ChromeScalars {
            loaded: ctx.tick().loaded,
            paused: ctx.tick().paused,
            settings_open: host.settings_open(),
            language: ctx.settings().shared.general.language,
            active_slot: None,
        };
        assert!(!host.chrome.cheap_same(&scalars));
        host.sync_menu_state(&ctx);
        let ctx = host.frame_ctx();
        let scalars = ChromeScalars {
            loaded: ctx.tick().loaded,
            paused: ctx.tick().paused,
            settings_open: host.settings_open(),
            language: ctx.settings().shared.general.language,
            active_slot: None,
        };
        assert!(host.chrome.cheap_same(&scalars));
        assert!(host.chrome.menu_same(ctx.slots(), ctx.shortcuts()));
        let replaced: Arc<[_]> = ctx.shortcuts().to_vec().into();
        assert!(!host.chrome.menu_same(ctx.slots(), &replaced));
        assert!(!host.chrome.title_same("nerust"));
        let tick = ctx.tick();
        host.refresh_window_title(&tick);
        let title = host.session_mut().window_title_cached(&tick).to_owned();
        assert!(host.chrome.title_same(&title));
    }
}
