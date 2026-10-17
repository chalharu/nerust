//! In-frame notification OSD sync. Session outcomes already land in
//! the shell `NotificationCenter` via `run_command`; this module drains
//! them into `OsdManager`, rasterizes the layer, and hands it to the
//! renderer. Error batches surface as Toast. The loop never blocks,
//! so unlike desktop dialogs there is no auto-pause.

use std::time::Instant;

use nerust_gui_shell::session::osd::{
    OSD_STATUS_RGB, OsdSync, OsdSyncInput, strip_pixels, strip_rows,
};
use nerust_render_traits::{ReservedStrip, SurfaceSize, renderer::OsdLayer};
use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};

use super::AndroidFrontend;

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
            top: strip_pixels(client_h, top_rows),
            bottom: strip_pixels(client_h, bottom_rows),
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
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => renderer.set_osd_layer(None),
            OsdSync::Render(plan) => {
                renderer.set_osd_layer(plan_layer(&plan, &mut self.osd_raster));
            }
        }
    }
}

fn plan_layer(
    plan: &nerust_gui_shell::session::osd::OsdPlan,
    raster: &mut nerust_render_osd::OsdRaster,
) -> Option<OsdLayer> {
    let rows = plan
        .rows
        .iter()
        .map(|row| nerust_render_osd::OsdRow {
            tag: row.tag.to_string(),
            text: row.text.clone(),
            rgb: row.rgb,
        })
        .collect();
    let status = plan
        .status
        .as_ref()
        .map(|(label, value)| nerust_render_osd::OsdStatus {
            label: label.clone(),
            value: value.clone(),
            rgb: OSD_STATUS_RGB,
        });
    let content = nerust_render_osd::OsdContent {
        rows,
        status,
        position: plan.position,
        status_position: plan.status_position,
    };
    raster.rasterize(&content, plan.width, plan.height)
}
