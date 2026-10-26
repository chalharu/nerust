//! One capture per entry point: settings, tick, slots, shortcuts, plus the OSD frame.
//! Static chrome is `Arc`-cached; per-tick liveness is read off the session, never cloned.

use std::sync::Arc;

/// One capture per pass: static chrome shared by `Arc`, dynamic tick read live.
pub struct FrameCtx {
    chrome: Arc<ChromeSnapshot>,
    tick: nerust_gui_shell::session::osd::TickMetrics,
}

struct ChromeSnapshot {
    generation: u64,
    settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>,
    system: Option<Box<dyn nerust_core_traits::identity::SystemId>>,
    slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
    shortcuts: Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    frame: FrameSnapshot,
}

impl FrameCtx {
    /// Pass generation for trace logging.
    pub fn generation(&self) -> u64 {
        self.chrome.generation
    }

    /// Settings snapshot for chrome and target-size math.
    pub fn settings(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot {
        &self.chrome.settings
    }

    /// Tick liveness for pause/load and title gates.
    pub fn tick(&self) -> nerust_gui_shell::session::osd::TickMetrics {
        self.tick
    }

    /// Immutable slot summaries for the menu identity gate.
    pub fn slots(&self) -> &Arc<[nerust_persistence::model::StateSlotSummary]> {
        &self.chrome.slots
    }

    /// Immutable shortcut bindings for the menu identity gate.
    pub fn shortcuts(&self) -> &Arc<[nerust_settings_core::model::input::ShortcutBinding]> {
        &self.chrome.shortcuts
    }

    /// OSD frame slice: notifications plus language.
    pub fn frame(&self) -> &FrameSnapshot {
        &self.chrome.frame
    }
}

/// One settings snapshot per entry point, threaded by reference.
/// Derived OSD values ride along so per-frame callers never rebuild them.
#[derive(Debug, Clone)]
pub struct FrameSnapshot {
    settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>,
    chrome: nerust_gui_shell::session::osd::OsdChrome,
    policy: nerust_gui_shell::session::osd::TransientPolicy,
}

impl FrameSnapshot {
    pub fn new(settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>) -> Self {
        use nerust_gui_shell::session::osd::{OsdChrome, TransientPolicy};
        let chrome = OsdChrome::from(&settings.local.notifications);
        let policy = TransientPolicy::from(&settings.local.notifications);
        Self {
            settings,
            chrome,
            policy,
        }
    }

    pub fn language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.settings.shared.general.language
    }

    /// Cached chrome: no per-frame rebuild.
    pub fn chrome(&self) -> &nerust_gui_shell::session::osd::OsdChrome {
        &self.chrome
    }

    pub fn settings_arc(&self) -> &Arc<nerust_gui_runtime::settings::SettingsSnapshot> {
        &self.settings
    }

    /// Cached transient policy: no per-frame rebuild.
    pub fn policy(&self) -> &nerust_gui_shell::session::osd::TransientPolicy {
        &self.policy
    }
}

/// Versioned frame cache: hot passes reuse the cached `Arc`.
#[derive(Default)]
pub struct FrameCache {
    entry: Option<Arc<ChromeSnapshot>>,
    generation: u64,
}

impl FrameCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fresh metrics per pass; only immutable chrome is cached.
    pub fn frame_ctx(&mut self, session: &nerust_gui_shell::session::SessionHandle) -> FrameCtx {
        self.capture(
            session.settings_arc(),
            session.slots_arc(),
            session.active_system_id(),
            session.tick_metrics(),
        )
    }

    fn capture(
        &mut self,
        settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>,
        slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
        system: Option<&dyn nerust_core_traits::identity::SystemId>,
        tick: nerust_gui_shell::session::osd::TickMetrics,
    ) -> FrameCtx {
        // Owners retain these allocations until content changes. Keeping the old
        // Arcs alive also prevents allocation-address reuse (ABA).
        let cached = self.entry.as_ref().filter(|cached| {
            Arc::ptr_eq(&cached.settings, &settings)
                && Arc::ptr_eq(&cached.slots, &slots)
                && cached.system.as_deref() == system
        });
        let chrome = if let Some(cached) = cached {
            Arc::clone(cached)
        } else {
            let shortcuts = self
                .entry
                .as_ref()
                .and_then(|cached| {
                    Arc::ptr_eq(&cached.settings, &settings).then(|| Arc::clone(&cached.shortcuts))
                })
                .unwrap_or_else(|| settings.shared.input.shortcuts.keyboard.clone().into());
            self.generation += 1;
            let fresh = Arc::new(ChromeSnapshot {
                generation: self.generation,
                frame: FrameSnapshot::new(Arc::clone(&settings)),
                settings,
                system: system.map(|id| id.clone_box()),
                slots,
                shortcuts,
            });
            log::trace!("frame cache miss: generation {}", fresh.generation);
            self.entry = Some(Arc::clone(&fresh));
            fresh
        };
        FrameCtx { chrome, tick }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::{registry::SystemRegistry, session::SessionHandle};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct TestSession {
        session: SessionHandle,
        root: PathBuf,
    }

    impl TestSession {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "nerust-host-frame-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).expect("isolated frame test directory");
            let session = SessionHandle::new_with_settings_paths(
                HostBackendCapabilities {
                    window: HostWindowCapabilities {
                        remembers_window_size: false,
                        supports_fullscreen_default: true,
                        supports_scaling: true,
                    },
                    presentation: None,
                },
                Arc::new(SystemRegistry::new(vec![])),
                Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
                SettingsPaths {
                    settings_file: root.join("settings.toml"),
                    central_storage_root: root.join("storage"),
                },
            )
            .expect("coreless frame test session");
            Self { session, root }
        }
    }

    impl Drop for TestSession {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn frame_cache_reuses_chrome_with_fresh_tick() {
        let support = TestSession::new();
        let mut cache = FrameCache::new();
        let first = cache.frame_ctx(&support.session);
        let tick = nerust_gui_shell::session::osd::TickMetrics {
            fps: 59.5,
            frame_counter: first.tick().frame_counter + 1,
            loaded: true,
            paused: true,
        };
        let second = cache.capture(
            Arc::clone(&first.chrome.settings),
            Arc::clone(first.slots()),
            None,
            tick,
        );
        assert!(Arc::ptr_eq(&first.chrome, &second.chrome));
        assert_eq!(second.tick().frame_counter, tick.frame_counter);
        assert_eq!(second.tick().fps, tick.fps);
        assert!(second.tick().loaded);
        assert!(second.tick().paused);
        assert!(!first.tick().loaded);
    }

    #[test]
    fn frame_cache_reuses_arc_on_hot_path() {
        let support = TestSession::new();
        let mut cache = FrameCache::new();
        let first = cache.frame_ctx(&support.session);
        let second = cache.frame_ctx(&support.session);
        assert!(Arc::ptr_eq(&first.chrome, &second.chrome));
        assert_eq!(first.generation(), second.generation());
    }

    #[test]
    fn frame_cache_bumps_on_settings_change() {
        let mut support = TestSession::new();
        let mut cache = FrameCache::new();
        let first = cache.frame_ctx(&support.session);
        let mut snapshot = support.session.settings_snapshot().clone();
        snapshot.shared.general.language =
            nerust_settings_core::model::language::AppLanguage::Japanese;
        support
            .session
            .apply_settings(snapshot)
            .expect("headless settings apply");
        let second = cache.frame_ctx(&support.session);
        assert!(!Arc::ptr_eq(&first.chrome, &second.chrome));
        assert!(second.generation() > first.generation());
    }

    #[test]
    fn frame_cache_tracks_notification_position_color_duration_and_area() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode, NotificationColor, NotificationPosition, NotificationSettings,
        };
        let changes: [fn(&mut NotificationSettings); 4] = [
            |settings| settings.position = NotificationPosition::TopRight,
            |settings| settings.info_color = NotificationColor::Cyan,
            |settings| settings.info_duration_ms += 1,
            |settings| settings.area_mode = NotificationAreaMode::Strip,
        ];
        let mut support = TestSession::new();
        let mut cache = FrameCache::new();
        for change in changes {
            let first = cache.frame_ctx(&support.session);
            let mut settings = first.settings().clone();
            change(&mut settings.local.notifications);
            support
                .session
                .apply_settings(settings.clone())
                .expect("headless settings apply");
            let second = cache.frame_ctx(&support.session);
            assert!(!Arc::ptr_eq(&first.chrome, &second.chrome));
            assert_eq!(second.frame().language(), settings.shared.general.language);
            assert!(Arc::ptr_eq(
                &second.chrome.settings,
                second.frame().settings_arc()
            ));
        }
    }

    #[test]
    fn frame_cache_tracks_subsecond_slot_replacement_without_rescanning_shortcuts() {
        use nerust_persistence::model::StateSlotSummary;
        let support = TestSession::new();
        let mut cache = FrameCache::new();
        let settings = support.session.settings_arc();
        let slot = StateSlotSummary {
            schema_version: 1,
            slot_id: 1,
            path: nerust_persistence::sidecar::StorageLocation::Native("state.slot".into()),
            saved_at: std::time::UNIX_EPOCH + std::time::Duration::from_millis(100),
            has_thumbnail: false,
            emulator_version: "test".into(),
        };
        let first = cache.capture(
            Arc::clone(&settings),
            vec![slot.clone()].into(),
            None,
            Default::default(),
        );
        let mut changed = slot;
        changed.saved_at += std::time::Duration::from_millis(1);
        let second = cache.capture(
            settings,
            vec![changed.clone()].into(),
            None,
            Default::default(),
        );
        assert!(second.generation() > first.generation());
        assert_eq!(second.slots()[0].saved_at, changed.saved_at);
        assert!(Arc::ptr_eq(first.shortcuts(), second.shortcuts()));
        // Previously captured immutable chrome remains unchanged.
        assert_ne!(first.slots()[0].saved_at, changed.saved_at);
    }
}
