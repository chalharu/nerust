//! OSD notification orchestration: drain, modal flow, overlay sync,
//! and strip computation. Display state lives in `OsdManager`
//! (`crate::aux::osd`); this impl block wires it to the session,
//! dialogs, overlay window, and renderer.

use std::time::Instant;

use nerust_gui_shell::session::commands::SessionCommand;
use nerust_render_traits::ReservedStrip;
use tao::event::WindowEvent;

use super::HostState;
use crate::ports::WindowOpener;
use nerust_gui_shell::session::osd::{OsdManager, OsdSnapshot, strip_pixels, strip_rows};

impl HostState {
    fn notification_settings(&self) -> nerust_settings_core::model::local::NotificationSettings {
        self.session.settings_snapshot().local.notifications.clone()
    }

    fn notification_language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.session.settings_snapshot().shared.general.language
    }

    /// Main client rect in physical pixels: outer frame minus
    /// decorations (inner_position() goes stale on some X11 WMs).
    fn main_client_rect(&self) -> Option<((i32, i32), (f64, f64))> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        let scale = window.scale_factor();
        let outer = window.outer_position().ok()?;
        let outer_size = window.outer_size();
        let side = outer_size.width.saturating_sub(size.width) / 2;
        let top = outer_size
            .height
            .saturating_sub(size.height)
            .saturating_sub(side);
        Some((
            (outer.x + side as i32, outer.y + top as i32),
            (size.width as f64 / scale, size.height as f64 / scale),
        ))
    }

    /// Create the overlay window tracking the main client area.
    /// Transparency refusal degrades to no OSD (log + history remain).
    pub(crate) fn ensure_osd_window(&mut self, opener: &dyn WindowOpener) {
        if self.osd_window.is_some() {
            return;
        }
        let Some((position, size)) = self.main_client_rect() else {
            return;
        };
        self.osd_window = opener.create_osd(position, size);
    }

    /// Reposition the overlay when the main geometry changed.
    /// Polled (not only event-driven): WM-driven moves/resizes do not
    /// reliably surface as tao events on every backend, and a missed
    /// event would strand the overlay forever. Cheap: two X queries
    /// per loop pass, skipped when the overlay is absent.
    pub(crate) fn sync_osd_geometry(&mut self) {
        let window = self.window.as_ref();
        let size = window.map(|window| window.inner_size());
        let scale = window.map(|window| window.scale_factor());
        let Some((position, logical_size)) = self.main_client_rect() else {
            log::debug!("OSD geometry: client rect unavailable");
            return;
        };
        let (size, scale) = match (size, scale) {
            (Some(size), Some(scale)) => (size, scale),
            _ => return,
        };
        log::debug!(
            "OSD geometry: inner={}x{} size={}x{} scale={scale} outer_size={}x{}",
            position.0,
            position.1,
            size.width,
            size.height,
            self.window
                .as_ref()
                .map(|window| window.outer_size().width)
                .unwrap_or(0),
            self.window
                .as_ref()
                .map(|window| window.outer_size().height)
                .unwrap_or(0),
        );
        let current = (
            position.0,
            position.1,
            size.width,
            size.height,
            scale.to_bits(),
        );
        if self.osd_geometry == Some(current) {
            return;
        }
        self.osd_geometry = Some(current);
        if let Some(handle) = self.osd_window.as_mut() {
            handle.move_to(position, logical_size);
        }
    }

    pub(crate) fn is_osd_window(&self, id: tao::window::WindowId) -> bool {
        self.osd_window
            .as_ref()
            .is_some_and(|handle| handle.shell.window.id() == id)
    }

    pub(crate) fn is_history_window(&self, id: tao::window::WindowId) -> bool {
        self.history_window
            .as_ref()
            .is_some_and(|handle| handle.shell.window.id() == id)
    }

    pub(crate) fn on_osd_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Destroyed) {
            self.osd_window = None;
            return;
        }
        if let Some(handle) = self.osd_window.as_mut() {
            crate::aux::shell::AuxWindow::pump_window_event(handle, event);
        }
    }

    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        if self.history_window.is_some() {
            self.sync_history_window();
            return;
        }
        if let Some(handle) = opener.create_history(self.notification_language()) {
            self.history_window = Some(handle);
            self.sync_history_window();
        }
    }

    fn sync_history_window(&mut self) {
        let language = self.notification_language();
        let lines: Vec<String> = self
            .session
            .notification_history()
            .map(|n| n.kind.text(language))
            .collect();
        if let Some(handle) = self.history_window.as_mut() {
            handle.sync(lines);
        }
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Destroyed) {
            self.history_window = None;
            return;
        }
        if let Some(handle) = self.history_window.as_mut() {
            crate::aux::shell::AuxWindow::pump_window_event(handle, event);
        }
    }

    /// Reserved edge strips in physical pixels. Static per settings
    /// (not per frame) so the renderer never churns surfaces.
    pub(crate) fn reserved_strip(&self) -> ReservedStrip {
        let settings = self.notification_settings();
        let client_h = self
            .window
            .as_ref()
            .map(|window| window.inner_size().height)
            .unwrap_or(0);
        let (top_rows, bottom_rows) = strip_rows(
            settings.area_mode,
            settings.position,
            settings.status_position,
            settings.status_enabled,
        );
        ReservedStrip {
            top: strip_pixels(client_h, top_rows),
            bottom: strip_pixels(client_h, bottom_rows),
        }
    }

    /// Drain shell notifications into display state, show aggregated
    /// error modals (auto-pausing first), and refresh the overlay.
    /// Returns the next expiry for loop wakeups.
    pub(crate) fn poll_notifications(&mut self) -> Option<Instant> {
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
            // Auto-pause first (spec): the game must not advance
            // behind the modal. Pause stays on after close; the user
            // resumes explicitly. A no-core Pause no-ops quietly.
            let _ = self.session.run_command(SessionCommand::Pause);
            let detail = modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.dialogs
                .show_error(self.notification_language(), &detail);
        }
        self.sync_osd_snapshot();
        self.sync_history_window();
        self.osd.next_deadline()
    }

    fn sync_osd_snapshot(&mut self) {
        if self.osd_window.is_none() {
            return;
        }
        let settings = self.notification_settings();
        let language = self.notification_language();
        let metrics = self.session.metrics();
        let status = settings
            .status_enabled
            .then(|| OsdManager::status_text(metrics.emulation_fps, metrics.paused, language));
        let client_h = self
            .window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                size.height as f64 / window.scale_factor()
            })
            .unwrap_or(0.0);
        let rows = self.osd.rows();
        let handle = self.osd_window.as_mut().expect("checked above");
        handle.sync(OsdSnapshot {
            rows,
            status,
            position: settings.position,
            status_position: settings.status_position,
            font_px: (client_h * 0.022) as f32,
        });
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd_window.as_ref()?;
        self.osd.next_deadline()
    }
}
