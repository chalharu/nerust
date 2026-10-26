use super::*;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{registry::SystemRegistry, session::access::PauseControl};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) struct Fixture {
    pub core: ApplicationHost,
    loader: Box<dyn nerust_gui_shell::load::RomLoader>,
    root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nerust-application-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let registry = Arc::new(SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
        ]));
        let loader = registry.create_loader(Default::default()).unwrap();
        let session = SessionHandle::new_with_settings_paths(
            HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
            registry,
            Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
            SettingsPaths::from_root(root.clone()),
        )
        .unwrap();
        Self {
            core: ApplicationHost::new(session),
            loader,
            root,
        }
    }
    pub fn load(&mut self) {
        let mut rom = vec![0u8; 16 + 16384 + 8192];
        rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
        rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
        for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
            rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
        }
        let path = self.root.join("loop.nes");
        std::fs::write(&path, rom).unwrap();
        self.core.load_path(self.loader.as_mut(), &path).unwrap();
        assert!(self.core.tick().loaded);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.core.begin_close();
        self.core.close_debug_flights();
        self.core.finish_close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn focus_modal_and_explicit_pause_compose_across_actual_media_replacement() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.set_pause_owner(PauseOwner::Modal, true);
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    assert!(fixture.core.owns_pause(PauseOwner::Focus));
    fixture.load();
    assert!(fixture.core.tick().paused);
    assert!(fixture.core.note_main_arrival());
    assert!(fixture.core.tick().paused, "modal still owns pause");
    fixture.core.pause();
    fixture.core.set_pause_owner(PauseOwner::Modal, false);
    assert!(fixture.core.tick().paused, "explicit pause survives ACK");
    fixture.core.resume();
    assert!(!fixture.core.tick().paused);
}

#[test]
fn settings_cancel_and_apply_preserve_modal_focus_and_user_owners() {
    let mut fixture = Fixture::new();
    assert!(
        fixture
            .core
            .dispatch(SessionEvent::SettingsOpened)
            .unwrap()
            .settings_changed
    );
    fixture.load();
    assert!(fixture.core.tick().paused);
    assert_eq!(fixture.core.tick().frame_counter, 0);
    assert!(
        !fixture
            .core
            .dispatch(SessionEvent::SettingsOpened)
            .unwrap()
            .settings_changed
    );
    fixture.core.set_pause_owner(PauseOwner::Modal, true);
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    let settings = fixture.core.settings_snapshot().clone();
    fixture.core.apply_settings(settings).unwrap();
    fixture.load();
    fixture
        .core
        .dispatch(SessionEvent::SettingsOpenFailed)
        .unwrap();
    assert!(!fixture.core.owns_pause(PauseOwner::Settings));
    assert!(fixture.core.owns_pause(PauseOwner::Modal));
    assert!(fixture.core.owns_pause(PauseOwner::Focus));
    fixture.core.pause();
    fixture.core.focus_event(FocusEvent::Main(true));
    fixture.core.set_pause_owner(PauseOwner::Modal, false);
    assert!(fixture.core.tick().paused);
    fixture.core.resume();
    assert!(!fixture.core.tick().paused);
    assert!(
        !fixture
            .core
            .dispatch(SessionEvent::SettingsClosed)
            .unwrap()
            .settings_changed
    );
}

#[test]
fn session_verbs_stay_total_headless() {
    let mut fixture = Fixture::new();
    fixture.core.pause();
    fixture.core.resume_session();
    assert!(!fixture.core.tick().loaded);
    assert!(!fixture.core.pause_held());
}

#[test]
fn unarmed_burst_end_commits_nothing() {
    let mut fixture = Fixture::new();
    fixture.core.tick_focus_sync();
    assert!(!fixture.core.note_main_arrival());
    assert!(!fixture.core.pause_held());
}

#[test]
fn armed_burst_end_without_core_stays_paused_off() {
    let mut fixture = Fixture::new();
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    assert!(!fixture.core.note_main_arrival());
    assert!(!fixture.core.pause_held());
}

#[test]
fn focus_effects_commit_before_capture_and_modal_ack_changes_late_truth() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.note_main_departure();
    fixture.core.tick_focus_sync();
    let captured = fixture.core.frame_ctx();
    assert!(captured.tick().paused);
    fixture.core.set_pause_owner(PauseOwner::Modal, true);
    fixture
        .core
        .focus_event(FocusEvent::Aux(AuxKind::Debug, true));
    assert!(fixture.core.tick().paused);
    fixture.core.set_pause_owner(PauseOwner::Modal, false);
    assert!(!fixture.core.tick().paused);
    assert!(captured.tick().paused, "old capture is not liveness truth");
}

#[test]
fn application_close_flushes_exactly_once_after_resource_teardown() {
    let mut fixture = Fixture::new();
    fixture.load();
    fixture.core.request_debug_open();
    fixture.core.request_ppu_open();
    fixture.core.set_live_wanted(true);
    let flushes = std::cell::Cell::new(0);
    let flush = |session: &mut SessionHandle| {
        flushes.set(flushes.get() + 1);
        session.flush_before_exit();
    };
    fixture.core.finish_close_with(flush);
    assert!(!fixture.core.flushed, "flush waits for close entry");
    assert!(fixture.core.begin_close());
    assert!(!fixture.core.window_usable());
    fixture.core.close_debug_flights();
    assert!(!fixture.core.take_debug_open_request());
    assert!(!fixture.core.take_ppu_open_request());
    assert!(!fixture.core.live_armed().load(Ordering::Acquire));
    assert!(!fixture.core.debug_pending());
    fixture.core.finish_close_with(flush);
    assert!(fixture.core.flushed);
    assert_eq!(flushes.get(), 1);
    // Deliberately re-arm state: a duplicate close must not run the teardown again.
    fixture.core.set_pause_owner(PauseOwner::Modal, true);
    assert!(!fixture.core.begin_close());
    fixture.core.finish_close_with(flush);
    assert_eq!(flushes.get(), 1);
    assert!(fixture.core.owns_pause(PauseOwner::Modal));
}

#[test]
fn chrome_gates_skip_repeat_pushes_and_retain_source_allocations() {
    let mut fixture = Fixture::new();
    let first = fixture.core.frame_ctx();
    let sig = fixture.core.menu_update(&first, false).unwrap();
    assert!(!sig.loaded);
    assert!(fixture.core.menu_update(&first, false).is_none());
    assert!(Arc::ptr_eq(&sig.shortcuts, first.shortcuts()));
    assert!(
        fixture
            .core
            .menu_update(&first, true)
            .unwrap()
            .settings_open
    );
    let replacement: Arc<[_]> = first.shortcuts().to_vec().into();
    assert!(!Arc::ptr_eq(&replacement, &sig.shortcuts));
    assert!(!fixture.core.title_same("nerust"));
    let title = fixture.core.title_update(&first.tick()).unwrap();
    assert!(fixture.core.title_same(&title));
    assert!(fixture.core.title_update(&first.tick()).is_none());
    assert_eq!(fixture.core.frame_ctx().generation(), first.generation());
    let mut settings = fixture.core.settings_snapshot().clone();
    settings.local.notifications.info_duration_ms += 1;
    fixture.core.apply_settings(settings).unwrap();
    let replaced = fixture.core.frame_ctx();
    assert!(!Arc::ptr_eq(first.shortcuts(), replaced.shortcuts()));
    assert!(fixture.core.menu_update(&replaced, true).is_some());
}

#[test]
fn history_gate_tracks_sequence_language_and_reopen_without_native_state() {
    use nerust_settings_core::model::language::AppLanguage;
    let mut fixture = Fixture::new();
    assert!(fixture.core.history_key().is_none());
    assert!(fixture.core.history_update(AppLanguage::English).is_some());
    assert!(fixture.core.history_update(AppLanguage::English).is_none());
    fixture.core.report_host_error("history entry".into());
    assert!(
        !fixture
            .core
            .history_update(AppLanguage::English)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.core.history_update(AppLanguage::Japanese).is_some());
    fixture.core.clear_history();
    assert!(
        fixture
            .core
            .history_update(AppLanguage::Japanese)
            .unwrap()
            .is_empty()
    );
    fixture.core.invalidate_history();
    assert!(fixture.core.history_update(AppLanguage::Japanese).is_some());
    fixture.core.begin_close();
    assert!(fixture.core.history_key().is_none());
}

#[test]
fn render_retry_and_notification_pause_are_owned_by_application() {
    let mut fixture = Fixture::new();
    assert!(fixture.core.render_ready());
    fixture.core.render_failed("test render failure".into());
    assert!(!fixture.core.render_ready());
    let deadline = fixture.core.render_retry_deadline().unwrap();
    assert!(
        !fixture
            .core
            .take_render_retry(deadline - std::time::Duration::from_nanos(1))
    );
    assert!(fixture.core.take_render_retry(deadline));
    fixture.core.render_succeeded();
    assert!(fixture.core.render_ready());
    let mut notifications = OsdNotifications::new();
    let ctx = fixture.core.frame_ctx();
    let report = fixture
        .core
        .pump_notifications(&mut notifications, &ctx, Instant::now());
    assert!(report.modal_queued);
    assert!(fixture.core.owns_pause(PauseOwner::Modal));
}
