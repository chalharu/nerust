//! In-frame notification OSD sync. Session outcomes already land in
//! the shell `NotificationCenter` via `run_command`; this module drains
//! them into `OsdManager`, rasterizes the layer, and hands it to the
//! renderer. Error batches surface as Toast. The loop never blocks,
//! so unlike desktop dialogs there is no auto-pause.

use std::time::Instant;

use nerust_gui_shell::session::osd::{
    OsdPresenter, OsdSync, OsdSyncInput, strip_pixels, strip_rows,
};
use nerust_render_traits::{ReservedStrip, SurfaceSize, renderer::SafeInsets};
use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};

use super::AndroidFrontend;

/// Display-density scale for the in-frame OSD (desktop verifies at
/// 1.0; phone screens halve it). Shared with strip reservation.
pub(super) const OSD_UI_SCALE: f32 = 0.5;

/// Platform font roots invisible to fontdb's system scan.
pub(super) const ANDROID_FONT_DIRS: [&str; 2] = ["/system/fonts", "/product/fonts"];

/// Corner-radius and edge margin, in dp. Physical rounded corners
/// clip surface-corner pills, so every side steps inward by this.
const EDGE_SAFE_DP: f32 = 24.0;

/// Extra inset along the letterbox axis (top/bottom in portrait,
/// left/right in landscape), in dp. That axis has dead bar space,
/// so the shift stays readable while clearing notches and corners.
const BAR_AXIS_EXTRA_DP: f32 = 16.0;

impl AndroidFrontend {
    pub(super) fn notification_settings(&self) -> NotificationSettings {
        self.session.settings_snapshot().local.notifications.clone()
    }

    pub(super) fn notification_language(&self) -> AppLanguage {
        self.session.settings_snapshot().shared.general.language
    }

    pub(super) fn reserved_strip(&self, client_h: u32) -> ReservedStrip {
        let settings = self.notification_settings();
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
    /// dp-based so the physical clearance is density-independent;
    /// the bar axis (dead space) takes the extra shift.
    pub(super) fn safe_insets(&self, size: SurfaceSize) -> SafeInsets {
        let density = self
            .window
            .as_ref()
            .map(|window| window.scale_factor() as f32)
            .unwrap_or(1.0)
            .max(0.5);
        let base = (EDGE_SAFE_DP * density).round() as u32;
        let extra = (BAR_AXIS_EXTRA_DP * density).round() as u32;
        // Surface relatively taller than the game: bars sit top and
        // bottom, so the vertical shift has room. Without a loaded
        // ROM fall back to surface orientation.
        let taller = match self.session.render_profile() {
            Some(profile)
                if profile.physical_size.width > 0.0 && profile.physical_size.height > 0.0 =>
            {
                size.height as f32 * profile.physical_size.width
                    > size.width as f32 * profile.physical_size.height
            }
            _ => size.height >= size.width,
        };
        if taller {
            SafeInsets {
                top: base + extra,
                right: base,
                bottom: base + extra,
                left: base,
            }
        } else {
            SafeInsets {
                top: base,
                right: base + extra,
                bottom: base,
                left: base + extra,
            }
        }
    }

    /// Drain shell notifications into display state. Error batches go
    /// to Toast; OSD changes request a redraw. Safe to call every loop
    /// pass; cheap when idle.
    pub(super) fn poll_notifications(&mut self) {
        let now = Instant::now();
        let drained = self.session.drain_notifications();
        if !drained.is_empty() {
            let settings = self.notification_settings();
            let language = self.notification_language();
            self.osd.push_all(drained, &settings, language, now);
        }
        self.osd.poll(now);
        let modal = self.osd.take_modal();
        if !modal.is_empty() {
            let detail = modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            super::show_toast(&self.app, &detail);
        }
        if self.osd.dirty() {
            self.request_redraw();
        }
    }

    /// OSD expiry deadline for the event-loop wait, if any labels are visible.
    pub(super) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.next_deadline()
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Also applies the reserved edge strip when settings or surface
    /// size change it. No-op without a renderer.
    pub(super) fn sync_osd_layer(&mut self, size: SurfaceSize) {
        if self.renderer.is_none() {
            return;
        }
        let insets = self.safe_insets(size);
        if insets != self.osd_last_insets {
            self.osd_last_insets = insets;
            self.osd_raster.set_safe_insets(insets);
            // The presenter gates on size/status only, so force a
            // re-evaluation when the safe area moves the boxes.
            self.osd_presenter = OsdPresenter::new();
        }
        let strip = self.reserved_strip(size.height);
        if strip != self.osd_last_strip {
            self.osd_last_strip = strip;
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_reserved_strip(strip);
            }
        }
        let settings = self.notification_settings();
        let language = self.notification_language();
        let metrics = self.session.metrics();
        let decision = self.osd_presenter.sync(
            &mut self.osd,
            OsdSyncInput {
                settings: &settings,
                language,
                fps: metrics.emulation_fps,
                paused: metrics.paused,
                now: Instant::now(),
                width: size.width,
                height: size.height,
            },
        );
        // Re-borrow after the session reads above; it only lives
        // for the layer handoff.
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => renderer.set_osd_layer(None),
            OsdSync::Render(plan) => {
                let content = plan.content();
                renderer.set_osd_layer(self.osd_raster.rasterize(
                    &content,
                    plan.width,
                    plan.height,
                ));
            }
        }
    }
}
