//! In-frame notification OSD sync: drain, rasterize, hand to the renderer.
//! Error batches surface as a modal dialog; the loop never blocks.

use std::time::Instant;

use nerust_gui_shell::session::{
    SessionHandle,
    osd::{OsdManager, OsdPresenter, OsdSync, OsdSyncInput},
};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize,
    physical::PhysicalSize,
    renderer::{RenderPipeline, SafeInsets, strip_pixels},
};
use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};

/// Display-density scale for the in-frame OSD (desktop verifies at
/// 1.0; phone screens halve it). Shared with strip reservation.
const OSD_UI_SCALE: f32 = 0.5;

/// Platform font roots invisible to fontdb's system scan.
const ANDROID_FONT_DIRS: [&str; 2] = ["/system/fonts", "/product/fonts"];

/// Corner-radius and edge margin, in dp. Physical rounded corners
/// clip surface-corner pills, so every side steps inward by this.
const EDGE_SAFE_DP: f32 = 24.0;

/// Extra inset along the letterbox axis, in dp.
/// That axis has dead bar space, so the shift stays readable.
const BAR_AXIS_EXTRA_DP: f32 = 16.0;

/// Display state: notification queue, rasterizer, and strip sync.
/// Fonts and the manager survive an `invalidate`.
struct OsdState {
    manager: OsdManager,
    raster: nerust_render_osd::OsdRaster,
    presenter: OsdPresenter,
    last_strip: ReservedStrip,
}

impl OsdState {
    fn new() -> Self {
        let mut raster = nerust_render_osd::OsdRaster::with_font_dirs(&ANDROID_FONT_DIRS);
        raster.set_ui_scale(OSD_UI_SCALE);
        Self {
            manager: OsdManager::new(),
            raster,
            presenter: OsdPresenter::new(),
            last_strip: ReservedStrip::default(),
        }
    }

    /// Forget sync state after a renderer rebuild.
    fn invalidate(&mut self) {
        self.presenter.invalidate();
        self.last_strip = ReservedStrip::default();
    }
}

/// Java window-insets cache: size, density, and push generation.
/// Stale pushes never re-query; the render thread never blocks on JNI.
struct InsetsCache {
    size: SurfaceSize,
    scale_key: u32,
    version: u64,
    system: SafeInsets,
}

impl InsetsCache {
    fn new() -> Self {
        Self {
            size: SurfaceSize::new(0, 0),
            scale_key: u32::MAX,
            version: u64::MAX,
            system: SafeInsets::default(),
        }
    }

    /// Forget size and push tracking; the next frame re-reads the cache.
    fn invalidate(&mut self) {
        *self = Self::new();
    }

    /// Cached push or default: default-until-push, never a blocking query.
    fn refresh(&mut self, size: SurfaceSize, scale: f32) -> SafeInsets {
        let scale_key = crate::density::density_key(scale);
        let version = super::bridge::insets_version();
        if size != self.size || scale_key != self.scale_key || version != self.version {
            self.size = size;
            self.scale_key = scale_key;
            self.version = version;
            self.system = super::bridge::cached_system_insets()
                .map(|[top, right, bottom, left]| SafeInsets {
                    top: top.max(0) as u32,
                    right: right.max(0) as u32,
                    bottom: bottom.max(0) as u32,
                    left: left.max(0) as u32,
                })
                .unwrap_or_default();
        }
        self.system
    }
}

/// OSD display state plus the sync fingerprint. One field on the frontend.
/// Keeps raster, strip, insets, and the notification manager only.
pub(super) struct AndroidOsd {
    state: OsdState,
    insets: InsetsCache,
}

/// Per-frame sync context for the rasterizer handoff.
/// `now` is caller-injected so render and poll share one clock read.
pub(super) struct OsdSyncCtx<'a> {
    pub tick: nerust_gui_shell::session::osd::TickMetrics,
    pub settings: &'a NotificationSettings,
    pub language: AppLanguage,
    pub renderer: &'a mut dyn RenderPipeline,
    pub size: SurfaceSize,
    pub insets: SafeInsets,
    pub now: Instant,
}

impl AndroidOsd {
    pub(super) fn new() -> Self {
        Self {
            state: OsdState::new(),
            insets: InsetsCache::new(),
        }
    }

    /// Borrow the notification queue for the frontend-owned dialog coordinator.
    /// The borrow ends when the closure returns.
    pub(super) fn with_modal<R>(&mut self, operation: impl FnOnce(&mut OsdManager) -> R) -> R {
        operation(&mut self.state.manager)
    }

    /// Forget sync state after a renderer rebuild.
    /// Fonts and scale survive; insets re-read on the next frame.
    pub(super) fn invalidate(&mut self) {
        self.state.invalidate();
        self.insets.invalidate();
    }

    // Frontend-owned modal flow lives in `DialogCoordinator`; `with_modal`
    // is the only `&mut` path to the shell queue besides the shell pump.

    /// Drain shell notifications into display state. Error batches join
    /// the shell modal queue; the dialog shows one at a time.
    pub(super) fn poll(
        &mut self,
        session: &mut SessionHandle,
        settings: &NotificationSettings,
        language: AppLanguage,
        now: Instant,
        tick: nerust_gui_shell::session::osd::TickMetrics,
    ) -> bool {
        let report = session.pump_osd(
            &mut self.state.manager,
            nerust_gui_shell::session::osd::PushInput {
                policy: nerust_gui_shell::session::osd::TransientPolicy::from(settings),
                language,
                now,
                tick,
            },
        );
        report.changed
    }

    /// OSD expiry deadline for the event-loop wait, if any labels are visible.
    pub(super) fn wakeup(&self, now: Instant) -> Option<Instant> {
        self.state.manager.wakeup_deadline(now)
    }

    /// Reserved edge strips in physical pixels. Static per settings
    /// (not per frame) so the renderer never churns surfaces.
    pub(super) fn reserved_strip(settings: &NotificationSettings, client_h: u32) -> ReservedStrip {
        let (top_rows, bottom_rows) =
            nerust_gui_shell::session::osd::OsdChrome::from(settings).reserved_rows();
        ReservedStrip {
            top: strip_pixels(client_h, top_rows, OSD_UI_SCALE),
            bottom: strip_pixels(client_h, bottom_rows, OSD_UI_SCALE),
        }
    }

    /// Safe-area insets for rounded corners and the letterbox axis.
    /// `system` comes from the Java window-insets cache; zeros when unavailable.
    pub(super) fn safe_insets(
        scale_factor: f32,
        content: Option<PhysicalSize>,
        size: SurfaceSize,
        system: SafeInsets,
    ) -> SafeInsets {
        let density = scale_factor.max(0.5);
        let base = (EDGE_SAFE_DP * density).round() as u32;
        let extra = (BAR_AXIS_EXTRA_DP * density).round() as u32;
        // Bars sit top and bottom when the surface is taller than the game.
        // Without a loaded ROM fall back to surface orientation.
        let taller = match content {
            Some(content) if content.width > 0.0 && content.height > 0.0 => {
                size.height as f32 * content.width > size.width as f32 * content.height
            }
            _ => size.height >= size.width,
        };
        if taller {
            SafeInsets {
                top: base.max(system.top) + extra,
                right: base.max(system.right),
                bottom: base.max(system.bottom) + extra,
                left: base.max(system.left),
            }
        } else {
            SafeInsets {
                top: base.max(system.top),
                right: base.max(system.right) + extra,
                bottom: base.max(system.bottom),
                left: base.max(system.left) + extra,
            }
        }
    }

    /// Java window insets, refreshed only on size/density/push change.
    /// Default-until-push: no frame ever blocks on a JNI query.
    pub(super) fn system_insets(&mut self, size: SurfaceSize, scale: f32) -> SafeInsets {
        self.insets.refresh(size, scale)
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// `now` is caller-injected; render and poll share one clock read per tick.
    pub(super) fn sync(&mut self, ctx: OsdSyncCtx<'_>) {
        let OsdSyncCtx {
            tick,
            settings,
            language,
            renderer,
            size,
            insets,
            now,
        } = ctx;
        let strip = Self::reserved_strip(settings, size.height);
        if strip != self.state.last_strip {
            self.state.last_strip = strip;
            renderer.set_reserved_strip(strip);
        }
        let decision = self.state.presenter.sync(
            &self.state.manager,
            OsdSyncInput {
                tick,
                chrome: nerust_gui_shell::session::osd::OsdChrome::from(settings),
                language,
                now,
                width: size.width,
                height: size.height,
                ui_scale: OSD_UI_SCALE,
                safe_insets: insets.into(),
            },
        );
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => renderer.set_osd_layer(None),
            OsdSync::Render(plan) => {
                let (width, height) = (plan.width, plan.height);
                let content = plan.into_content();
                self.state.raster.set_safe_insets(insets);
                renderer.set_osd_layer(self.state.raster.rasterize(&content, width, height));
            }
        }
    }
}
