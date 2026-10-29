//! Toolkit-independent OSD orchestration from notification pump to renderer sinks.
//! Native hosts own dialogs; application pause arbitration stays outside the raster path.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    time::Instant,
};

use nerust_gui_shell::session::{
    SessionHandle,
    osd::{OsdChrome, OsdManager, OsdPresenter, OsdSync, OsdSyncInput, PumpReport, PushInput},
};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize,
    renderer::{OsdLayer, OsdSink, StripSink, strip_pixels},
};
use wakui::support::cache::ChangeGate;

use crate::frame::FrameCtx;
use wakui::chrome::dialog::{ModalAdvance, ModalPresentation, ModalToken};
use wakui::flow::loop_driver::WakeNeed;

#[derive(Debug, Default)]
pub struct OsdTurn {
    pub modal: Option<ModalPresentation>,
    pub wake: WakeNeed,
    pub changed: bool,
}

/// Skip leaves the destination untouched; Clear explicitly removes its layer.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum OsdLayerUpdate {
    #[default]
    Skip,
    Clear,
    Set(OsdLayer),
}

/// Owned output difference. Apply outside the host borrow, in delivery order.
/// If the destination is lost or delivery is abandoned, invalidate before preparing again.
#[must_use]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OsdUpdate {
    pub strip: Option<ReservedStrip>,
    pub layer: OsdLayerUpdate,
}

impl OsdUpdate {
    /// Apply the owned difference to the renderer sinks directly. There is
    /// no visitor: raster vocabulary belongs to the renderer, not to views.
    pub fn apply<S: OsdSink + StripSink + ?Sized>(self, sink: &mut S) {
        if let Some(strip) = self.strip {
            sink.set_reserved_strip(strip);
        }
        match self.layer {
            OsdLayerUpdate::Skip => {}
            OsdLayerUpdate::Clear => sink.set_osd_layer(None),
            OsdLayerUpdate::Set(layer) => sink.set_osd_layer(Some(layer)),
        }
    }
}

#[derive(Default)]
enum ModalState {
    #[default]
    Idle,
    // Keep structured content in OsdManager until actual delivery.
    Deferred,
    Active {
        token: ModalToken,
        receiver: Option<Receiver<()>>,
    },
}

#[derive(Default)]
pub struct OsdNotifications {
    manager: OsdManager,
    modal: ModalState,
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
        let report = session.pump_osd(
            &mut self.manager,
            PushInput {
                policy: *ctx.frame().policy(),
                language: ctx.frame().language(),
                now,
                tick: ctx.tick(),
            },
        );
        self.defer_modal();
        report
    }

    pub fn manager(&self) -> &OsdManager {
        &self.manager
    }

    pub fn pending(&self) -> usize {
        self.manager.modal_pending()
    }

    fn defer_modal(&mut self) {
        if !matches!(self.modal, ModalState::Active { .. }) {
            self.modal = if self.pending() > 0 {
                ModalState::Deferred
            } else {
                ModalState::Idle
            };
        }
    }

    /// Includes deferred delivery and the whole ACK chain, not just queued batches.
    pub fn modal_pending(&self) -> bool {
        !matches!(self.modal, ModalState::Idle) || self.pending() > 0
    }

    /// Only receiver-backed flights need a periodic parked-loop ACK backstop.
    /// Native callback dialogs wake their UI loop themselves.
    pub fn flight_pending(&self) -> bool {
        matches!(
            self.modal,
            ModalState::Active {
                receiver: Some(_),
                ..
            }
        )
    }

    /// Commit the flight before returning: synchronous presentation/ACK may reenter.
    pub fn take_modal(&mut self) -> Option<ModalPresentation> {
        if !matches!(self.modal, ModalState::Deferred) {
            return None;
        }
        let Some(detail) = self.manager.take_modal_batch() else {
            self.modal = ModalState::Idle;
            return None;
        };
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let token = loop {
            let id = NEXT.load(Ordering::Relaxed);
            let next = id.checked_add(1).expect("modal token exhausted");
            if NEXT
                .compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break ModalToken(id);
            }
        };
        self.modal = ModalState::Active {
            token,
            receiver: None,
        };
        Some(ModalPresentation { token, detail })
    }

    /// Attach after starting a native worker. A synchronous ACK or stale completion
    /// cannot attach a receiver to the next dialog, nor replace its receiver.
    pub fn attach_modal_receiver(&mut self, token: ModalToken, done: Receiver<()>) -> bool {
        match &mut self.modal {
            ModalState::Active {
                token: active,
                receiver,
            } if *active == token && receiver.is_none() => {
                *receiver = Some(done);
                true
            }
            _ => false,
        }
    }

    /// Consume an ACK (including a disconnected native dialog thread) and chain
    /// the next batch before ApplicationHost releases pause ownership.
    pub(crate) fn acknowledge_modal(&mut self, token: ModalToken) -> ModalAdvance {
        if !matches!(self.modal, ModalState::Active { token: active, .. } if active == token) {
            return ModalAdvance::Ignored;
        }
        self.modal = ModalState::Idle;
        self.manager.modal_closed();
        self.defer_modal();
        match self.take_modal() {
            Some(presentation) => ModalAdvance::Present(presentation),
            None => ModalAdvance::Drained,
        }
    }

    pub(crate) fn poll_modal_ack(&mut self) -> Option<ModalAdvance> {
        let ModalState::Active {
            token,
            receiver: Some(receiver),
        } = &self.modal
        else {
            return None;
        };
        match receiver.try_recv() {
            Err(TryRecvError::Empty) => None,
            result => {
                if result.is_err() {
                    log::error!("error dialog thread died before ack");
                }
                let token = *token;
                Some(self.acknowledge_modal(token))
            }
        }
    }

    /// Retire callbacks/receivers and queued content during ordered host teardown.
    /// ApplicationHost owns teardown's pause reset; old tokens never match again.
    pub fn close(&mut self) {
        *self = Self::new();
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
    last_strip: ChangeGate<ReservedStrip>,
    last_sync_geometry: ChangeGate<(u32, u32, OsdChrome)>,
}

impl Default for OsdLayerSync {
    fn default() -> Self {
        Self {
            raster: nerust_render_osd::OsdRaster::new(),
            presenter: OsdPresenter::new(),
            layer_live: false,
            last_strip: ChangeGate::new(),
            last_sync_geometry: ChangeGate::new(),
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
    /// No native output is called while preparing this owned difference.
    pub fn prepare(
        &mut self,
        manager: &OsdManager,
        size: SurfaceSize,
        client_height: u32,
        ctx: &FrameCtx,
        now: Instant,
    ) -> OsdUpdate {
        let (top_rows, bottom_rows) = ctx.frame().chrome().reserved_rows();
        let strip = ReservedStrip {
            top: strip_pixels(client_height, top_rows, 1.0),
            bottom: strip_pixels(client_height, bottom_rows, 1.0),
        };
        let strip = self.last_strip.check(&strip).then_some(strip);
        OsdUpdate {
            strip,
            ..self.prepare_layer(manager, size, ctx, now)
        }
    }

    /// Rasterize only a changed plan, without reserving a strip or calling native code.
    pub fn prepare_layer(
        &mut self,
        manager: &OsdManager,
        size: SurfaceSize,
        ctx: &FrameCtx,
        now: Instant,
    ) -> OsdUpdate {
        let chrome = *ctx.frame().chrome();
        let geometry = (size.width, size.height, chrome);
        let geometry_changed = self.last_sync_geometry.check(&geometry);
        if !self.is_due(manager, chrome.status_enabled, geometry_changed) {
            return OsdUpdate::default();
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
        let layer = match decision {
            OsdSync::Skip => OsdLayerUpdate::Skip,
            OsdSync::Clear => {
                self.layer_live = false;
                OsdLayerUpdate::Clear
            }
            OsdSync::Render(plan) => {
                let (width, height) = (plan.width, plan.height);
                let layer = self.raster.rasterize(&plan.into_content(), width, height);
                self.layer_live = layer.is_some();
                match layer {
                    Some(layer) => OsdLayerUpdate::Set(layer),
                    None => OsdLayerUpdate::Clear,
                }
            }
        };
        OsdUpdate {
            layer,
            ..Default::default()
        }
    }

    /// Compatibility delegate. Owned host adapters use prepare/apply instead.
    pub fn render<S: OsdSink + StripSink + ?Sized>(
        &mut self,
        manager: &OsdManager,
        sink: &mut S,
        size: SurfaceSize,
        client_height: u32,
        ctx: &FrameCtx,
        now: Instant,
    ) {
        self.prepare(manager, size, client_height, ctx, now)
            .apply(sink);
    }

    /// Compatibility delegate for callers that reserve their strip separately.
    pub fn sync_layer<S: OsdSink + ?Sized>(
        &mut self,
        manager: &OsdManager,
        sink: &mut S,
        size: SurfaceSize,
        ctx: &FrameCtx,
        now: Instant,
    ) {
        let update = self.prepare_layer(manager, size, ctx, now);
        match update.layer {
            OsdLayerUpdate::Skip => {}
            OsdLayerUpdate::Clear => sink.set_osd_layer(None),
            OsdLayerUpdate::Set(layer) => sink.set_osd_layer(Some(layer)),
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

    use crate::session::pause_owners;
    use nerust_core_traits::audio::AudioBackendRegistry;
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
    use crate::{application::ApplicationHost, frame::FrameCache};

    struct Fixture {
        session: SessionHandle,
        root: TestRoot,
    }

    struct TestRoot(PathBuf);

    impl Fixture {
        fn new() -> Self {
            Self::with_nes(false)
        }

        fn with_nes(nes: bool) -> Self {
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
                Arc::new(SystemRegistry::new(if nes {
                    vec![nerust_nes_factory::NesFactory::boxed().into()]
                } else {
                    Vec::new()
                })),
                Arc::new(AudioBackendRegistry::new()),
                SettingsPaths::from_root(root.clone()),
            )
            .expect("independent OSD session");
            Self {
                session,
                root: TestRoot(root),
            }
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

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
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
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        let owners = [
            pause_owners::FOCUS,
            pause_owners::SETTINGS,
            pause_owners::USER,
        ];
        for owner in owners {
            host.set_pause_owner(owner, true);
        }
        pump_error(&mut host, &mut notifications, "first");
        assert!(host.owns_pause(pause_owners::MODAL));
        assert!(
            !notifications.flight_pending(),
            "callback delivery needs no polling"
        );
        let first = notifications.take_modal().unwrap();
        assert!(first.detail.contains("first"));
        pump_error(&mut host, &mut notifications, "second");
        assert_eq!(notifications.pending(), 1);
        assert!(
            notifications.take_modal().is_none(),
            "no overlapping dialogs"
        );
        assert_eq!(
            host.acknowledge_modal(&mut notifications, ModalToken(0)),
            ModalAdvance::Ignored
        );
        let ModalAdvance::Present(next) = host.acknowledge_modal(&mut notifications, first.token)
        else {
            panic!("ACK must chain the queued batch before releasing Modal");
        };
        assert!(next.detail.contains("second"));
        assert_ne!(first.token, next.token);
        assert!(host.owns_pause(pause_owners::MODAL));
        assert_eq!(
            host.acknowledge_modal(&mut notifications, first.token),
            ModalAdvance::Ignored
        );
        assert!(notifications.take_modal().is_none());
        assert_eq!(
            host.acknowledge_modal(&mut notifications, next.token),
            ModalAdvance::Drained
        );
        assert_eq!(
            host.acknowledge_modal(&mut notifications, next.token),
            ModalAdvance::Ignored
        );
        assert!(!host.owns_pause(pause_owners::MODAL));
        assert!(host.redraw_needed());
        assert!(!notifications.modal_pending());
        for owner in owners {
            assert!(host.owns_pause(owner));
        }
        assert!(host.pause_held());
    }

    fn pump_error(host: &mut ApplicationHost, notifications: &mut OsdNotifications, detail: &str) {
        host.report_host_error(detail.into());
        let ctx = host.frame_ctx();
        assert!(
            host.pump_notifications(notifications, &ctx, Instant::now())
                .modal_queued
        );
    }

    #[test]
    fn idle_pump_does_not_resume_independent_pause_and_modal_adopts_it() {
        use nerust_gui_shell::session::commands::SessionCommand;
        let fixture = Fixture::with_nes(true);
        let mut host = ApplicationHost::new(fixture.session);
        let root = fixture.root;
        let registry = SystemRegistry::new(vec![nerust_nes_factory::NesFactory::boxed().into()]);
        let mut loader = Arc::new(registry)
            .create_loader(Default::default())
            .unwrap();
        let mut rom = vec![0u8; 16 + 16384 + 8192];
        rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
        rom[16..19].copy_from_slice(&[0x4c, 0, 0x80]);
        for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
            rom[16 + vector..18 + vector].copy_from_slice(&[0, 0x80]);
        }
        let path = root.0.join("loop.nes");
        std::fs::create_dir_all(&root.0).unwrap();
        std::fs::write(&path, rom).unwrap();
        host.load_path(loader.as_mut(), &path).unwrap();
        host.session_mut()
            .run_command(SessionCommand::Pause)
            .unwrap();
        assert!(host.tick().paused);
        assert!(!host.pause_held());
        let mut notifications = OsdNotifications::new();
        let ctx = host.frame_ctx();
        host.pump_notifications(&mut notifications, &ctx, Instant::now());
        assert!(
            host.tick().paused,
            "idle OSD cannot resume an independent stop"
        );
        assert!(!host.pause_held(), "idle pump cannot adopt a pause either");
        pump_error(&mut host, &mut notifications, "adopt stopped core");
        let presentation = notifications.take_modal().unwrap();
        assert!(host.owns_pause(pause_owners::USER));
        assert_eq!(
            host.acknowledge_modal(&mut notifications, presentation.token),
            ModalAdvance::Drained
        );
        assert!(
            host.tick().paused,
            "Modal release preserves adopted explicit pause"
        );
        host.session_mut().unload().unwrap();
    }

    #[test]
    fn queued_modal_uses_current_language_and_filters_before_final_ack() {
        let mut fixture = Fixture::new();
        let mut frames = FrameCache::new();
        let mut notifications = OsdNotifications::new();
        let now = Instant::now();
        let mut token = None;
        for detail in ["first", "second"] {
            fixture
                .session
                .report_external_error(FailureOrigin::Selection, detail.into());
            let ctx = frames.frame_ctx(&fixture.session);
            notifications.pump(&mut fixture.session, &ctx, now);
            if detail == "first" {
                token = Some(notifications.take_modal().unwrap().token);
            }
        }
        fixture.settings(|settings| settings.shared.general.language = AppLanguage::Japanese);
        let ctx = frames.frame_ctx(&fixture.session);
        notifications.pump(&mut fixture.session, &ctx, now);
        let ModalAdvance::Present(next) = notifications.acknowledge_modal(token.unwrap()) else {
            panic!("queued batch must survive language switch");
        };
        assert!(
            next.detail.contains("選択"),
            "queued batch relocalizes: {next:?}"
        );
        fixture
            .session
            .report_external_error(FailureOrigin::Host, "third".into());
        notifications.pump(&mut fixture.session, &ctx, now);
        assert_eq!(notifications.pending(), 1);
        fixture.settings(|settings| settings.local.notifications.error_enabled = false);
        let off = frames.frame_ctx(&fixture.session);
        notifications.pump(&mut fixture.session, &off, now);
        assert_eq!(notifications.pending(), 0);
        assert_eq!(
            notifications.acknowledge_modal(next.token),
            ModalAdvance::Drained
        );
    }

    #[test]
    fn deferred_delivery_refilters_and_relocalizes_without_a_duplicate_queue() {
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        pump_error(&mut host, &mut notifications, "deferred");
        assert_eq!(
            notifications.pending(),
            1,
            "structured batch stays in shell"
        );
        let mut settings = host.settings_snapshot().clone();
        settings.shared.general.language = AppLanguage::Japanese;
        host.apply_settings(settings).unwrap();
        let ctx = host.frame_ctx();
        host.pump_notifications(&mut notifications, &ctx, Instant::now());
        let presentation = notifications.take_modal().unwrap();
        assert!(presentation.detail.contains("通知"), "{presentation:?}");
        host.acknowledge_modal(&mut notifications, presentation.token);
        pump_error(
            &mut host,
            &mut notifications,
            "filtered before presentation",
        );
        let mut settings = host.settings_snapshot().clone();
        settings.local.notifications.error_enabled = false;
        host.apply_settings(settings).unwrap();
        let ctx = host.frame_ctx();
        host.pump_notifications(&mut notifications, &ctx, Instant::now());
        assert!(notifications.take_modal().is_none());
        assert!(!notifications.modal_pending());
        assert!(!host.owns_pause(pause_owners::MODAL));
    }

    #[test]
    fn receiver_ack_and_disconnect_chain_once_and_reject_late_attachment() {
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        host.set_pause_owner(pause_owners::SETTINGS, true);
        pump_error(&mut host, &mut notifications, "worker first");
        let first = notifications.take_modal().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(notifications.attach_modal_receiver(first.token, receiver));
        assert!(notifications.flight_pending());
        assert!(
            host.poll_modal_ack(&mut notifications).is_none(),
            "empty receiver keeps flight"
        );
        let (_duplicate_sender, duplicate_receiver) = std::sync::mpsc::channel();
        assert!(!notifications.attach_modal_receiver(first.token, duplicate_receiver));
        pump_error(&mut host, &mut notifications, "worker second");
        sender.send(()).unwrap();
        sender.send(()).unwrap();
        let Some(ModalAdvance::Present(second)) = host.poll_modal_ack(&mut notifications) else {
            panic!("worker ACK must chain");
        };
        assert!(host.owns_pause(pause_owners::MODAL));
        assert!(
            host.poll_modal_ack(&mut notifications).is_none(),
            "duplicate message retired with receiver"
        );
        let (dead_sender, dead_receiver) = std::sync::mpsc::channel();
        drop(dead_sender);
        assert!(!notifications.attach_modal_receiver(first.token, dead_receiver));
        let (dead_sender, dead_receiver) = std::sync::mpsc::channel();
        drop(dead_sender);
        assert!(notifications.attach_modal_receiver(second.token, dead_receiver));
        assert_eq!(
            host.poll_modal_ack(&mut notifications),
            Some(ModalAdvance::Drained)
        );
        assert!(host.poll_modal_ack(&mut notifications).is_none());
        assert!(!notifications.flight_pending());
        assert!(!host.owns_pause(pause_owners::MODAL));
        assert!(host.owns_pause(pause_owners::SETTINGS));
    }

    #[test]
    fn synchronous_callback_can_reborrow_and_ack_before_receiver_attachment() {
        use std::{cell::RefCell, rc::Rc};
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        pump_error(&mut host, &mut notifications, "callback");
        let shared = Rc::new(RefCell::new((host, notifications)));
        let presentation = shared.borrow_mut().1.take_modal().unwrap();
        // Fake native present synchronously calls map/response, outside all borrows.
        let present = |presentation: ModalPresentation| {
            let mut borrowed = shared.borrow_mut();
            let (host, notifications) = &mut *borrowed;
            assert!(host.owns_pause(pause_owners::MODAL));
            assert!(notifications.take_modal().is_none());
            host.acknowledge_modal(notifications, presentation.token)
        };
        let token = presentation.token;
        assert_eq!(present(presentation), ModalAdvance::Drained);
        let mut borrowed = shared.borrow_mut();
        let (host, notifications) = &mut *borrowed;
        let (_sender, receiver) = std::sync::mpsc::channel();
        assert!(!notifications.attach_modal_receiver(token, receiver));
        assert_eq!(
            host.acknowledge_modal(notifications, token),
            ModalAdvance::Ignored
        );
        assert!(!host.owns_pause(pause_owners::MODAL));
    }

    #[test]
    fn disconnected_receiver_chains_before_releasing_modal() {
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        pump_error(&mut host, &mut notifications, "dead worker");
        let first = notifications.take_modal().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        notifications.attach_modal_receiver(first.token, receiver);
        pump_error(&mut host, &mut notifications, "after disconnect");
        drop(sender);
        let Some(ModalAdvance::Present(next)) = host.poll_modal_ack(&mut notifications) else {
            panic!("disconnect must chain just like ACK");
        };
        assert!(next.detail.contains("after disconnect"));
        assert!(host.owns_pause(pause_owners::MODAL));
        assert_eq!(
            host.acknowledge_modal(&mut notifications, next.token),
            ModalAdvance::Drained
        );
        assert!(!host.pause_held());
    }

    #[test]
    fn close_retires_deferred_flights_receivers_and_old_runtime_tokens() {
        let fixture = Fixture::new();
        let mut host = ApplicationHost::new(fixture.session);
        let _root = fixture.root;
        let mut notifications = OsdNotifications::new();
        pump_error(&mut host, &mut notifications, "old runtime");
        let old = notifications.take_modal().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        notifications.attach_modal_receiver(old.token, receiver);
        pump_error(&mut host, &mut notifications, "queued on close");
        notifications.close();
        assert!(sender.send(()).is_err());
        assert!(!notifications.modal_pending());
        assert!(notifications.take_modal().is_none());
        pump_error(&mut host, &mut notifications, "new runtime");
        let new = notifications.take_modal().unwrap();
        assert_ne!(new.token, old.token);
        assert_eq!(
            host.acknowledge_modal(&mut notifications, old.token),
            ModalAdvance::Ignored
        );
        assert!(host.owns_pause(pause_owners::MODAL));
        assert_eq!(
            host.acknowledge_modal(&mut notifications, new.token),
            ModalAdvance::Drained
        );
        pump_error(&mut host, &mut notifications, "deferred on close");
        notifications.close();
        assert!(notifications.take_modal().is_none());
        host.begin_close();
        assert!(!host.pause_held());
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
