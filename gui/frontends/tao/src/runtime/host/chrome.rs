//! Chrome sync: menu signature plus window title, each with its own gate.
//! Owned by `HostState`; title-only moves never touch the menu path.

use crate::util::Fnv1a;

/// Menu signature for the chrome dirty gate; `generation` fingerprints content.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MenuSig {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
    pub(super) slots: std::sync::Arc<[nerust_persistence::model::StateSlotSummary]>,
    pub(super) shortcuts: std::sync::Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    pub(super) generation: u64,
}

impl MenuSig {
    /// Cheap content fingerprint: FNV-1a over slot ids/timestamps and shortcut codes.
    pub(super) fn fingerprint(
        slots: &[nerust_persistence::model::StateSlotSummary],
        shortcuts: &[nerust_settings_core::model::input::ShortcutBinding],
    ) -> u64 {
        use std::hash::Hash;
        let mut hasher = Fnv1a::new();
        for slot in slots {
            slot.slot_id.hash(&mut hasher);
            if let Ok(age) = slot.saved_at.duration_since(std::time::UNIX_EPOCH) {
                age.as_secs().hash(&mut hasher);
            }
            slot.emulator_version.hash(&mut hasher);
        }
        for shortcut in shortcuts {
            shortcut.action.hash(&mut hasher);
            shortcut.key.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Scalar snapshot for the cheap chrome gate: title is gated separately.
pub(super) struct ChromeScalars {
    pub(super) loaded: bool,
    pub(super) paused: bool,
    pub(super) settings_open: bool,
    pub(super) language: nerust_settings_core::model::language::AppLanguage,
    pub(super) active_slot: Option<u64>,
    pub(super) slots_len: usize,
    pub(super) shortcuts_len: usize,
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

    /// Cheap scalar gate: a match falls through to the exact fingerprint/slice compare.
    pub(super) fn cheap_same(&self, scalars: &ChromeScalars) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            sig.loaded == scalars.loaded
                && sig.paused == scalars.paused
                && sig.settings_open == scalars.settings_open
                && sig.language == scalars.language
                && sig.active_slot == scalars.active_slot
                && sig.slots.len() == scalars.slots_len
                && sig.shortcuts.len() == scalars.shortcuts_len
        })
    }

    /// Exact gate: generation first, then slice compare. True means the menu IPC can be skipped.
    pub(super) fn menu_same(
        &self,
        generation: u64,
        slots: &[nerust_persistence::model::StateSlotSummary],
        shortcuts: &[nerust_settings_core::model::input::ShortcutBinding],
    ) -> bool {
        self.last_menu_sig.as_ref().is_some_and(|sig| {
            sig.generation == generation
                && slots == sig.slots.as_ref()
                && shortcuts == sig.shortcuts.as_ref()
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
    fn fingerprint_is_stable_and_content_sensitive() {
        let empty = MenuSig::fingerprint(&[], &[]);
        assert_eq!(empty, MenuSig::fingerprint(&[], &[]));
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        assert_eq!(empty, MenuSig::fingerprint(ctx.slots(), &[]));
        assert_ne!(empty, MenuSig::fingerprint(ctx.slots(), ctx.shortcuts()));
    }

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
            slots_len: ctx.slots().len(),
            shortcuts_len: ctx.shortcuts().len(),
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
            slots_len: ctx.slots().len(),
            shortcuts_len: ctx.shortcuts().len(),
        };
        assert!(host.chrome.cheap_same(&scalars));
        let fresh = MenuSig::fingerprint(ctx.slots(), ctx.shortcuts());
        assert!(host.chrome.menu_same(fresh, ctx.slots(), ctx.shortcuts()));
        assert!(
            !host
                .chrome
                .menu_same(fresh.wrapping_add(1), ctx.slots(), ctx.shortcuts())
        );
        assert!(!host.chrome.title_same("nerust"));
        let tick = ctx.tick();
        host.refresh_window_title(&tick);
        let title = host.session_mut().window_title_cached(&tick).to_owned();
        assert!(host.chrome.title_same(&title));
    }
}
