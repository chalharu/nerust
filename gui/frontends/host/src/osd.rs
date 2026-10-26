//! Toolkit-independent OSD orchestration from notification pump to renderer sinks.
//! Native hosts own dialogs and pause arbitration; neither belongs to the raster path.

use std::time::Instant;

use nerust_frontend_kernel::Versioned;
use nerust_gui_shell::session::{
    SessionHandle,
    osd::{OsdChrome, OsdManager, OsdPresenter, OsdSync, OsdSyncInput, PumpReport, PushInput},
};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize,
    renderer::{OsdSink, StripSink, strip_pixels},
};

use crate::frame::FrameCtx;

/// ACK outcome shared by threaded dialogs and native widget callbacks.
/// `Drained` releases only the host's Modal pause owner, never resumes directly.
/// Adapters handle this on the UI thread: release Modal on `Drained`, or
/// present the next dialog on `Present` without releasing the hold in between.
#[derive(Debug, PartialEq, Eq)]
pub enum ModalAdvance {
    Present(String),
    Drained,
}

#[derive(Default)]
pub struct OsdNotifications {
    manager: OsdManager,
}

impl OsdNotifications {
    pub fn new() -> Self {
        Self::default()
    }

    /// One capture supplies both pump policy and the subsequent layer sync.
    /// `modal_queued` acquires the Modal pause owner for the whole queue lifetime.
    pub fn pump(
        &mut self,
        session: &mut SessionHandle,
        ctx: &FrameCtx,
        now: Instant,
    ) -> PumpReport {
        session.pump_osd(
            &mut self.manager,
            PushInput {
                policy: *ctx.frame().policy(),
                language: ctx.frame().language(),
                now,
                tick: ctx.tick(),
            },
        )
    }

    pub fn manager(&self) -> &OsdManager {
        &self.manager
    }

    pub fn pending(&self) -> usize {
        self.manager.modal_pending()
    }

    /// The shell's open gate prevents a second dialog while an ACK is outstanding.
    pub fn take_modal_batch(&mut self) -> Option<String> {
        self.manager.take_modal_batch()
    }

    /// Consume an ACK (including a disconnected native dialog thread) and chain
    /// the next batch before allowing the native host to release pause ownership.
    pub fn acknowledge_modal(&mut self) -> ModalAdvance {
        self.manager.modal_closed();
        match self.take_modal_batch() {
            Some(detail) => ModalAdvance::Present(detail),
            None => ModalAdvance::Drained,
        }
    }

    pub fn wakeup_deadline(&self, now: Instant) -> Option<Instant> {
        self.manager.wakeup_deadline(now)
    }
}

/// Persistent render-side state, independent of the native surface lifetime.
/// Call `invalidate` whenever the destination loses its layer (replacement or
/// surface recreation). This is independent of the settings/frame generation;
/// resize and settings changes are already tracked by the presenter inputs.
pub struct OsdLayerSync {
    raster: nerust_render_osd::OsdRaster,
    presenter: OsdPresenter,
    layer_live: bool,
    last_strip: Versioned<ReservedStrip>,
    last_sync_geometry: Versioned<(u32, u32, OsdChrome)>,
}

impl Default for OsdLayerSync {
    fn default() -> Self {
        Self {
            raster: nerust_render_osd::OsdRaster::new(),
            presenter: OsdPresenter::new(),
            layer_live: false,
            last_strip: Versioned::new(),
            last_sync_geometry: Versioned::new(),
        }
    }
}

impl OsdLayerSync {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh destination has neither the previous layer nor its reserved strip.
    pub fn invalidate(&mut self) {
        self.presenter.invalidate();
        self.last_strip.clear();
        self.last_sync_geometry.clear();
        self.layer_live = false;
    }

    pub fn layer_live(&self) -> bool {
        self.layer_live
    }

    /// A live but steady layer is not itself a reason to run the presenter.
    pub fn is_due(
        &self,
        manager: &OsdManager,
        status_enabled: bool,
        geometry_changed: bool,
    ) -> bool {
        manager.generation() != self.presenter.last_seen()
            || manager.has_visible()
            || status_enabled
            || geometry_changed
    }

    /// Strip reservation uses physical client height, not the renderer's cropped extent.
    pub fn render<S: OsdSink + StripSink + ?Sized>(
        &mut self,
        manager: &OsdManager,
        sink: &mut S,
        size: SurfaceSize,
        client_height: u32,
        ctx: &FrameCtx,
        now: Instant,
    ) {
        let (top_rows, bottom_rows) = ctx.frame().chrome().reserved_rows();
        let strip = ReservedStrip {
            top: strip_pixels(client_height, top_rows, 1.0),
            bottom: strip_pixels(client_height, bottom_rows, 1.0),
        };
        if self.last_strip.check(&strip) {
            sink.set_reserved_strip(strip);
        }
        self.sync_layer(manager, sink, size, ctx, now);
    }

    /// Rasterize only a changed plan; Skip never touches the renderer sink.
    pub fn sync_layer<S: OsdSink + ?Sized>(
        &mut self,
        manager: &OsdManager,
        sink: &mut S,
        size: SurfaceSize,
        ctx: &FrameCtx,
        now: Instant,
    ) {
        let chrome = *ctx.frame().chrome();
        let geometry = (size.width, size.height, chrome);
        let geometry_changed = self.last_sync_geometry.check(&geometry);
        if !self.is_due(manager, chrome.status_enabled, geometry_changed) {
            return;
        }
        let decision = self.presenter.sync(
            manager,
            OsdSyncInput {
                tick: ctx.tick(),
                chrome,
                language: ctx.frame().language(),
                now,
                width: size.width,
                height: size.height,
                ui_scale: 1.0,
                safe_insets: Default::default(),
            },
        );
        log::trace!("osd sync geometry v{}", self.last_sync_geometry.version());
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => {
                self.layer_live = false;
                sink.set_osd_layer(None);
            }
            OsdSync::Render(plan) => {
                let (width, height) = (plan.width, plan.height);
                let layer = self.raster.rasterize(&plan.into_content(), width, height);
                self.layer_live = layer.is_some();
                sink.set_osd_layer(layer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_frontend_kernel::pause::{PauseOwner, PausePolicy};
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::{
        registry::SystemRegistry,
        session::{
            notification::{FailureOrigin, Message, NotificationLevel, UiNotification},
            osd::FrameChannel,
        },
    };
    use nerust_render_traits::renderer::OsdLayer;
    use nerust_settings_core::model::{
        language::AppLanguage,
        local::{NotificationAreaMode, NotificationPosition},
    };

    use super::*;
    use crate::frame::FrameCache;

    struct Fixture {
        session: SessionHandle,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "nerust-host-osd-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            let session = SessionHandle::new_with_settings_paths(
                HostBackendCapabilities {
                    window: HostWindowCapabilities {
                        remembers_window_size: false,
                        supports_fullscreen_default: true,
                        supports_scaling: true,
                    },
                    presentation: None,
                },
                Arc::new(SystemRegistry::new(Vec::new())),
                Arc::new(AudioBackendRegistry::new()),
                SettingsPaths::from_root(root.clone()),
            )
            .expect("independent OSD session");
            Self { session, root }
        }

        fn settings(
            &mut self,
            change: impl FnOnce(&mut nerust_gui_runtime::settings::SettingsSnapshot),
        ) {
            let mut settings = self.session.settings_snapshot().clone();
            change(&mut settings);
            self.session
                .apply_settings(settings)
                .expect("OSD settings apply");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[derive(Debug, Default)]
    struct Sink {
        strips: Vec<ReservedStrip>,
        layers: Vec<Option<OsdLayer>>,
    }

    impl OsdSink for Sink {
        fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
            self.layers.push(layer);
        }
    }

    impl StripSink for Sink {
        fn set_reserved_strip(&mut self, strip: ReservedStrip) {
            self.strips.push(strip);
        }
    }

    fn size() -> SurfaceSize {
        SurfaceSize::new(320, 240)
    }

    #[test]
    fn idle_pump_and_render_skip_layers_and_memoize_strip() {
        let mut fixture = Fixture::new();
        let mut frames = FrameCache::new();
        let ctx = frames.frame_ctx(&fixture.session);
        let mut notifications = OsdNotifications::new();
        let mut layer = OsdLayerSync::new();
        let mut sink = Sink::default();
        let now = Instant::now();
        let report = notifications.pump(&mut fixture.session, &ctx, now);
        assert!(!report.changed);
        assert!(!report.modal_queued);
        assert_eq!(notifications.wakeup_deadline(now), None);
        assert!(!layer.is_due(notifications.manager(), false, false));
        assert!(layer.is_due(notifications.manager(), true, false));
        assert!(layer.is_due(notifications.manager(), false, true));
        for _ in 0..2 {
            layer.render(notifications.manager(), &mut sink, size(), 480, &ctx, now);
        }
        assert_eq!(sink.strips, [ReservedStrip::default()]);
        assert!(sink.layers.is_empty());
        assert!(!layer.layer_live());
    }

    #[test]
    fn status_resize_zero_size_and_replacement_republish_only_when_needed() {
        let mut fixture = Fixture::new();
        fixture.settings(|settings| settings.local.notifications.status_enabled = true);
        let ctx = FrameCache::new().frame_ctx(&fixture.session);
        let manager = OsdManager::new();
        let mut layer = OsdLayerSync::new();
        let mut sink = Sink::default();
        let now = Instant::now();
        layer.render(&manager, &mut sink, size(), 480, &ctx, now);
        layer.render(&manager, &mut sink, size(), 480, &ctx, now);
        assert_eq!(sink.layers.len(), 1);
        assert!(sink.layers[0].is_some());
        assert!(layer.layer_live());
        layer.render(
            &manager,
            &mut sink,
            SurfaceSize::new(640, 480),
            480,
            &ctx,
            now,
        );
        assert_eq!(sink.layers.len(), 2);
        layer.render(&manager, &mut sink, SurfaceSize::new(0, 0), 480, &ctx, now);
        assert!(sink.layers[2].is_none());
        assert!(!layer.layer_live());
        layer.render(&manager, &mut sink, size(), 480, &ctx, now);
        assert!(sink.layers[3].is_some());
        layer.invalidate();
        assert!(!layer.layer_live());
        let mut fresh = Sink::default();
        layer.render(&manager, &mut fresh, size(), 480, &ctx, now);
        layer.render(&manager, &mut fresh, size(), 480, &ctx, now);
        assert_eq!(fresh.layers.len(), 1);
        assert_eq!(fresh.layers[0], sink.layers[0]);
        assert_eq!(fresh.strips, sink.strips);
        fixture.settings(|settings| settings.local.notifications.status_enabled = false);
        let off = FrameCache::new().frame_ctx(&fixture.session);
        layer.render(&manager, &mut fresh, size(), 480, &off, now);
        layer.render(&manager, &mut fresh, size(), 480, &off, now);
        assert_eq!(fresh.layers.len(), 2);
        assert!(fresh.layers[1].is_none());
    }

    #[test]
    fn strip_tracks_client_pixels_and_settings_not_cropped_surface_height() {
        let mut fixture = Fixture::new();
        fixture.settings(|settings| {
            let notifications = &mut settings.local.notifications;
            notifications.position = NotificationPosition::TopLeft;
            notifications.area_mode = NotificationAreaMode::Strip;
            notifications.status_enabled = true;
            notifications.status_position = NotificationPosition::BottomRight;
            notifications.status_area_mode = NotificationAreaMode::Strip;
        });
        let mut frames = FrameCache::new();
        let ctx = frames.frame_ctx(&fixture.session);
        let mut layer = OsdLayerSync::new();
        let manager = OsdManager::new();
        let mut sink = Sink::default();
        let now = Instant::now();
        for height in [480, 480, 960] {
            layer.render(&manager, &mut sink, size(), height, &ctx, now);
        }
        assert_eq!(sink.strips.len(), 2);
        for (strip, height) in sink.strips.iter().zip([480, 960]) {
            assert_eq!(strip.top, strip_pixels(height, 1, 1.0));
            assert_eq!(strip.bottom, strip.top);
        }
        fixture.settings(|settings| {
            settings.local.notifications.area_mode = NotificationAreaMode::Overlay;
            settings.local.notifications.status_area_mode = NotificationAreaMode::Overlay;
        });
        let overlay = frames.frame_ctx(&fixture.session);
        layer.render(&manager, &mut sink, size(), 960, &overlay, now);
        assert_eq!(sink.strips[2], ReservedStrip::default());
    }

    #[test]
    fn modal_pump_chains_ack_without_releasing_other_pause_owners() {
        let mut fixture = Fixture::new();
        let mut frames = FrameCache::new();
        let mut notifications = OsdNotifications::new();
        let now = Instant::now();
        let mut pause = PausePolicy::default();
        pause.set(PauseOwner::Focus, true, false);
        for detail in ["first", "second"] {
            fixture
                .session
                .report_external_error(FailureOrigin::Host, detail.into());
            let ctx = frames.frame_ctx(&fixture.session);
            let report = notifications.pump(&mut fixture.session, &ctx, now);
            assert!(report.modal_queued);
            pause.set(PauseOwner::Modal, report.modal_queued, true);
            if detail == "first" {
                assert!(notifications.take_modal_batch().unwrap().contains(detail));
            }
        }
        assert_eq!(notifications.pending(), 1);
        assert!(
            notifications.take_modal_batch().is_none(),
            "no overlapping dialogs"
        );
        let ModalAdvance::Present(next) = notifications.acknowledge_modal() else {
            panic!("ACK must chain the queued batch before releasing Modal");
        };
        assert!(next.contains("second"));
        assert!(pause.owns(PauseOwner::Modal));
        assert!(notifications.take_modal_batch().is_none());
        assert_eq!(notifications.acknowledge_modal(), ModalAdvance::Drained);
        assert!(
            pause.set(PauseOwner::Modal, false, true),
            "Focus still owns pause"
        );
        assert!(!pause.owns(PauseOwner::Modal));
        assert!(pause.owns(PauseOwner::Focus));
    }

    #[test]
    fn queued_modal_uses_current_language_and_filters_before_final_ack() {
        let mut fixture = Fixture::new();
        let mut frames = FrameCache::new();
        let mut notifications = OsdNotifications::new();
        let now = Instant::now();
        for detail in ["first", "second"] {
            fixture
                .session
                .report_external_error(FailureOrigin::Selection, detail.into());
            let ctx = frames.frame_ctx(&fixture.session);
            notifications.pump(&mut fixture.session, &ctx, now);
            if detail == "first" {
                notifications.take_modal_batch().unwrap();
            }
        }
        fixture.settings(|settings| settings.shared.general.language = AppLanguage::Japanese);
        let ctx = frames.frame_ctx(&fixture.session);
        notifications.pump(&mut fixture.session, &ctx, now);
        let ModalAdvance::Present(next) = notifications.acknowledge_modal() else {
            panic!("queued batch must survive language switch");
        };
        assert!(next.contains("選択"), "queued batch relocalizes: {next}");
        fixture
            .session
            .report_external_error(FailureOrigin::Host, "third".into());
        notifications.pump(&mut fixture.session, &ctx, now);
        assert_eq!(notifications.pending(), 1);
        fixture.settings(|settings| settings.local.notifications.error_enabled = false);
        let off = frames.frame_ctx(&fixture.session);
        notifications.pump(&mut fixture.session, &off, now);
        assert_eq!(notifications.pending(), 0);
        assert_eq!(notifications.acknowledge_modal(), ModalAdvance::Drained);
    }

    #[test]
    fn transient_generation_and_expiry_reach_renderer_and_wake_gate() {
        let fixture = Fixture::new();
        let ctx = FrameCache::new().frame_ctx(&fixture.session);
        let now = Instant::now();
        let mut notifications = OsdNotifications::new();
        notifications.manager.push_all_with_policy(
            [UiNotification {
                level: NotificationLevel::Info,
                message: Message::paused(),
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            ctx.frame().policy(),
            ctx.frame().language(),
            now,
            FrameChannel::Composited,
        );
        notifications.manager.poll(now);
        let deadline = notifications
            .wakeup_deadline(now)
            .expect("visible row expires");
        let mut layer = OsdLayerSync::new();
        let mut sink = Sink::default();
        layer.render(notifications.manager(), &mut sink, size(), 480, &ctx, now);
        layer.render(notifications.manager(), &mut sink, size(), 480, &ctx, now);
        assert_eq!(sink.layers.len(), 1);
        assert!(sink.layers[0].is_some());
        let expired = deadline + Duration::from_millis(1);
        notifications.manager.poll(expired);
        assert!(layer.is_due(notifications.manager(), false, false));
        assert_eq!(notifications.wakeup_deadline(expired), None);
        layer.render(
            notifications.manager(),
            &mut sink,
            size(),
            480,
            &ctx,
            expired,
        );
        layer.render(
            notifications.manager(),
            &mut sink,
            size(),
            480,
            &ctx,
            expired,
        );
        assert_eq!(sink.layers.len(), 2);
        assert!(sink.layers[1].is_none());
        assert!(!layer.layer_live());
    }
}
