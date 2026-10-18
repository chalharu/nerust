//! In-frame notification OSD sync. Session outcomes already land in
//! the shell `NotificationCenter` via `run_command`; this module drains
//! them into `OsdManager`, rasterizes the layer, and hands it to the
//! renderer. Error batches surface as a modal dialog (frontend-driven
//! via `take_pending_errors`); the loop never blocks, so no auto-pause.

use std::time::Instant;

use nerust_gui_shell::session::{
    SessionHandle,
    osd::{OsdManager, OsdPresenter, OsdSync, OsdSyncInput, strip_rows},
};
use nerust_render_traits::{
    ReservedStrip, SurfaceSize,
    physical::PhysicalSize,
    renderer::{GpuRenderer, SafeInsets, strip_pixels},
};
use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};
use winit::platform::android::activity::AndroidApp;

/// Display-density scale for the in-frame OSD (desktop verifies at
/// 1.0; phone screens halve it). Shared with strip reservation.
const OSD_UI_SCALE: f32 = 0.5;

/// Platform font roots invisible to fontdb's system scan.
const ANDROID_FONT_DIRS: [&str; 2] = ["/system/fonts", "/product/fonts"];

/// Corner-radius and edge margin, in dp. Physical rounded corners
/// clip surface-corner pills, so every side steps inward by this.
const EDGE_SAFE_DP: f32 = 24.0;

/// Extra inset along the letterbox axis (top/bottom in portrait,
/// left/right in landscape), in dp. That axis has dead bar space,
/// so the shift stays readable while clearing notches and corners.
const BAR_AXIS_EXTRA_DP: f32 = 16.0;

/// OSD display state plus the sync fingerprint. One field on the
/// frontend; everything OSD lives here.
pub(super) struct AndroidOsd {
    manager: OsdManager,
    raster: nerust_render_osd::OsdRaster,
    presenter: OsdPresenter,
    last_strip: ReservedStrip,
    last_system_size: SurfaceSize,
    last_system_scale: f32,
    last_system: SafeInsets,
}

/// Per-frame sync context. Bundled so the rasterizer handoff
/// stays narrow as new dimensions (insets, scale) arrive.
pub(super) struct OsdSyncCtx<'a> {
    pub session: &'a SessionHandle,
    pub settings: &'a NotificationSettings,
    pub language: AppLanguage,
    pub renderer: &'a mut dyn GpuRenderer,
    pub size: SurfaceSize,
    pub insets: SafeInsets,
}

impl AndroidOsd {
    pub(super) fn new() -> Self {
        let mut raster = nerust_render_osd::OsdRaster::with_font_dirs(&ANDROID_FONT_DIRS);
        raster.set_ui_scale(OSD_UI_SCALE);
        Self {
            manager: OsdManager::new(),
            raster,
            presenter: OsdPresenter::new(),
            last_strip: ReservedStrip::default(),
            last_system_size: SurfaceSize::new(0, 0),
            last_system_scale: 0.0,
            last_system: SafeInsets::default(),
        }
    }

    /// Forget sync state after a renderer rebuild: the new object
    /// holds no layer or strip to skip against. Fonts and scale
    /// survive; insets re-query on the next frame via cleared key.
    pub(super) fn reset(&mut self) {
        self.presenter.invalidate();
        self.last_strip = ReservedStrip::default();
        self.last_system_size = SurfaceSize::new(0, 0);
    }

    /// Take the next error batch for the modal dialog, if none is up.
    /// Shell `ModalBox` gates on its open flag and caps the queue.
    pub(super) fn take_modal_batch(&mut self) -> Option<String> {
        self.manager.take_modal_batch()
    }

    /// Release the shell modal gate after the dialog is dismissed.
    pub(super) fn modal_closed(&mut self) {
        self.manager.modal_closed();
    }

    /// Queued error batches still awaiting display.
    pub(super) fn modal_pending(&self) -> usize {
        self.manager.modal_pending()
    }

    /// Drain shell notifications into display state. Error batches join
    /// the shell modal queue; the dialog shows one at a time.
    pub(super) fn poll(
        &mut self,
        session: &mut SessionHandle,
        settings: &NotificationSettings,
        language: AppLanguage,
        now: Instant,
    ) -> bool {
        let (drained, channel) = session.drain_routed();
        if !drained.is_empty() {
            self.manager
                .push_all(drained, settings, language, now, channel);
        }
        self.manager.poll(now);
        if let Some(detail) = self.manager.take_modal_text() {
            self.manager.queue_modal(detail);
        }
        self.manager.dirty()
    }

    /// OSD expiry deadline for the event-loop wait, if any labels are visible.
    pub(super) fn wakeup(&self) -> Option<Instant> {
        self.manager.next_deadline()
    }

    /// Reserved edge strips in physical pixels. Static per settings
    /// (not per frame) so the renderer never churns surfaces.
    pub(super) fn reserved_strip(settings: &NotificationSettings, client_h: u32) -> ReservedStrip {
        let (top_rows, bottom_rows) = strip_rows(
            settings.area_mode,
            settings.position,
            settings.status_area_mode,
            settings.status_position,
            settings.status_enabled,
        );
        ReservedStrip {
            top: strip_pixels(client_h, top_rows, OSD_UI_SCALE),
            bottom: strip_pixels(client_h, bottom_rows, OSD_UI_SCALE),
        }
    }

    /// Safe-area insets for rounded corners and the letterbox axis.
    /// dp-based corner base plus measured OS insets: physical
    /// clearance is density-independent, and the bar axis (dead
    /// space) takes the extra shift. `system` comes from the Java
    /// window-insets cache; zeros when the bridge is unavailable.
    pub(super) fn safe_insets(
        scale_factor: f32,
        content: Option<PhysicalSize>,
        size: SurfaceSize,
        system: SafeInsets,
    ) -> SafeInsets {
        let density = scale_factor.max(0.5);
        let base = (EDGE_SAFE_DP * density).round() as u32;
        let extra = (BAR_AXIS_EXTRA_DP * density).round() as u32;
        // Surface relatively taller than the game: bars sit top and
        // bottom, so the vertical shift has room. Without a loaded
        // ROM fall back to surface orientation.
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

    /// Java window insets, re-queried only on size/density change.
    /// Per-frame JNI is the cost; IME-without-resize stays a known gap.
    pub(super) fn system_insets(
        &mut self,
        app: &AndroidApp,
        size: SurfaceSize,
        scale: f32,
    ) -> SafeInsets {
        if size != self.last_system_size || scale != self.last_system_scale {
            self.last_system_size = size;
            self.last_system_scale = scale;
            self.last_system = Self::query_system_insets(app);
        }
        self.last_system
    }

    /// Root-window safe insets in physical pixels, cached on the Java
    /// side and refreshed on every layout pass. Falls back to zeros
    /// (corner dp base still applies) when the bridge is unavailable.
    pub(super) fn query_system_insets(app: &AndroidApp) -> SafeInsets {
        let insets: Option<[i32; 4]> = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as _) }
            .attach_current_thread(|env| {
                let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
                let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
                let values = env.call_method(
                    &activity,
                    jni::jni_str!("getSafeInsetsPx"),
                    jni::jni_sig!("()[I"),
                    &[],
                )?;
                let array = env.cast_local::<jni::objects::JIntArray>(values.l()?)?;
                let mut out = [0i32; 4];
                array.get_region(env, 0, &mut out)?;
                Ok::<_, jni::errors::Error>(out)
            })
            .ok();
        match insets {
            Some([top, right, bottom, left]) => SafeInsets {
                top: top.max(0) as u32,
                right: right.max(0) as u32,
                bottom: bottom.max(0) as u32,
                left: left.max(0) as u32,
            },
            None => {
                log::debug!("system insets unavailable; using dp heuristics");
                SafeInsets::default()
            }
        }
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Also applies the reserved edge strip when settings or surface
    /// size change it.
    pub(super) fn sync(&mut self, ctx: OsdSyncCtx<'_>) {
        let OsdSyncCtx {
            session,
            settings,
            language,
            renderer,
            size,
            insets,
        } = ctx;
        let strip = Self::reserved_strip(settings, size.height);
        if strip != self.last_strip {
            self.last_strip = strip;
            renderer.set_reserved_strip(strip);
        }
        let metrics = session.metrics();
        let decision = self.presenter.sync(
            &mut self.manager,
            OsdSyncInput {
                settings,
                language,
                fps: metrics.emulation_fps,
                paused: metrics.paused,
                now: Instant::now(),
                width: size.width,
                height: size.height,
                ui_scale: OSD_UI_SCALE,
                safe_insets: insets,
            },
        );
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => renderer.set_osd_layer(None),
            OsdSync::Render(plan) => {
                let content = plan.content();
                self.raster.set_safe_insets(insets);
                renderer.set_osd_layer(self.raster.rasterize(&content, plan.width, plan.height));
            }
        }
    }
}
