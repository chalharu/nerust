//! One capture per entry point: settings, tick, slots, shortcuts, plus the OSD frame.
//! Static chrome is `Arc`-cached; per-tick liveness is read off the session, never cloned.

use std::sync::Arc;

use crate::util::Fnv1a;

/// One capture per pass: static chrome shared by `Arc`, dynamic tick read live.
pub(crate) struct FrameCtx {
    generation: u64,
    settings: nerust_gui_runtime::settings::SettingsSnapshot,
    tick: nerust_gui_shell::session::osd::TickMetrics,
    slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
    shortcuts: Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    frame: FrameSnapshot,
}

impl FrameCtx {
    /// Pass generation for trace logging.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Settings snapshot for chrome and target-size math.
    pub(super) fn settings(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot {
        &self.settings
    }

    /// Tick liveness for pause/load and title gates.
    pub(super) fn tick(&self) -> nerust_gui_shell::session::osd::TickMetrics {
        self.tick
    }

    /// Slot summaries for menu fingerprinting.
    pub(super) fn slots(&self) -> &Arc<[nerust_persistence::model::StateSlotSummary]> {
        &self.slots
    }

    /// Shortcut bindings for menu fingerprinting.
    pub(super) fn shortcuts(&self) -> &Arc<[nerust_settings_core::model::input::ShortcutBinding]> {
        &self.shortcuts
    }

    /// OSD frame slice: notifications plus language.
    pub(super) fn frame(&self) -> &FrameSnapshot {
        &self.frame
    }
}

/// Static chrome key: scalars plus allocation-free content hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChromeKey {
    loaded: bool,
    paused: bool,
    active_slot: Option<u64>,
    slots_hash: u64,
    shortcuts_hash: u64,
    language: nerust_settings_core::model::language::AppLanguage,
    fullscreen_default: bool,
    scaling: nerust_settings_core::model::local::ScalingMode,
    notifications_hash: u64,
}

impl ChromeKey {
    pub(super) fn capture(session: &nerust_gui_shell::session::SessionHandle) -> Self {
        use std::hash::Hash;
        let tick = session.tick_metrics();
        let mut slots = Fnv1a::new();
        for slot in session.slots() {
            slot.slot_id.hash(&mut slots);
            if let Ok(age) = slot.saved_at.duration_since(std::time::UNIX_EPOCH) {
                age.as_secs().hash(&mut slots);
            }
        }
        let snapshot = session.settings_snapshot();
        let mut shortcuts = Fnv1a::new();
        for binding in snapshot.shared.input.shortcuts.keyboard.iter() {
            binding.action.hash(&mut shortcuts);
            binding.key.hash(&mut shortcuts);
        }
        let mut notifications = Fnv1a::new();
        snapshot
            .local
            .notifications
            .status_enabled
            .hash(&mut notifications);
        snapshot
            .local
            .notifications
            .info_enabled
            .hash(&mut notifications);
        snapshot
            .local
            .notifications
            .error_modal
            .hash(&mut notifications);
        Self {
            loaded: tick.loaded,
            paused: tick.paused,
            active_slot: session.active_slot_id(),
            slots_hash: slots.finish(),
            shortcuts_hash: shortcuts.finish(),
            language: snapshot.shared.general.language,
            fullscreen_default: snapshot.local.video.window.fullscreen_default,
            scaling: snapshot.local.video.window.scaling,
            notifications_hash: notifications.finish(),
        }
    }
}

impl FrameCtx {
    pub(super) fn capture(
        generation: u64,
        session: &nerust_gui_shell::session::SessionHandle,
    ) -> Self {
        let settings = session.settings_snapshot().clone();
        let tick = session.tick_metrics();
        let slots: Arc<[nerust_persistence::model::StateSlotSummary]> =
            session.slots().to_vec().into();
        let shortcuts: Arc<[nerust_settings_core::model::input::ShortcutBinding]> =
            settings.shared.input.shortcuts.keyboard.clone().into();
        let frame = FrameSnapshot::capture(session);
        Self {
            generation,
            settings,
            tick,
            slots,
            shortcuts,
            frame,
        }
    }
}

/// One settings snapshot per entry point, threaded by reference.
#[derive(Debug, Clone)]
pub(crate) struct FrameSnapshot(std::sync::Arc<FrameSnapshotData>);

#[derive(Debug)]
struct FrameSnapshotData {
    notifications: nerust_settings_core::model::local::NotificationSettings,
    language: nerust_settings_core::model::language::AppLanguage,
}

impl FrameSnapshot {
    pub(super) fn capture(session: &nerust_gui_shell::session::SessionHandle) -> Self {
        let snapshot = session.settings_snapshot();
        Self(std::sync::Arc::new(FrameSnapshotData {
            notifications: snapshot.local.notifications.clone(),
            language: snapshot.shared.general.language,
        }))
    }

    pub(crate) fn notifications(
        &self,
    ) -> &nerust_settings_core::model::local::NotificationSettings {
        &self.0.notifications
    }

    pub(crate) fn language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.0.language
    }
}

/// Versioned frame cache: hot passes reuse the cached `Arc`.
pub(super) struct FrameCache {
    entry: Option<(ChromeKey, Arc<FrameCtx>)>,
    generation: u64,
}

impl FrameCache {
    pub(super) fn new() -> Self {
        Self {
            entry: None,
            generation: 0,
        }
    }

    /// One capture per pass: hot passes reuse the cached `Arc`.
    pub(super) fn frame_ctx(
        &mut self,
        session: &nerust_gui_shell::session::SessionHandle,
    ) -> Arc<FrameCtx> {
        let key = ChromeKey::capture(session);
        if let Some((cached_key, cached)) = self.entry.as_ref()
            && *cached_key == key
        {
            return Arc::clone(cached);
        }
        self.generation += 1;
        let fresh = Arc::new(FrameCtx::capture(self.generation, session));
        log::trace!("frame cache miss: generation {}", fresh.generation());
        self.entry = Some((key, Arc::clone(&fresh)));
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::ChromeKey;
    use crate::runtime::host::TestHost;

    #[test]
    fn chrome_key_repeats_on_steady_session() {
        let support = TestHost::new();
        let host = support.host;
        assert_eq!(
            ChromeKey::capture(host.session()),
            ChromeKey::capture(host.session())
        );
    }

    #[test]
    fn chrome_key_covers_content_not_ticks() {
        // The key holds no tick liveness by construction, so steady
        // sessions repeat while playback bumps `frame_counter`.
        let support = TestHost::new();
        let host = support.host;
        let key = ChromeKey::capture(host.session());
        assert_eq!(key, ChromeKey::capture(host.session()));
    }

    #[test]
    fn frame_cache_reuses_arc_on_hot_path() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let second = host.frame_ctx();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(first.generation(), second.generation());
    }

    #[test]
    fn frame_cache_bumps_on_settings_change() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let mut snapshot = host.session().settings_snapshot().clone();
        snapshot.shared.general.language =
            nerust_settings_core::model::language::AppLanguage::Japanese;
        host.apply_settings(snapshot)
            .expect("headless settings apply");
        let second = host.frame_ctx();
        assert!(!std::sync::Arc::ptr_eq(&first, &second));
        assert!(second.generation() > first.generation());
    }
}
