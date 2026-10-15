//! GTK OSD notifications: overlay labels, error modals, history.
//!
//! Display policy (queue folding, expiry, burst caps, strips) lives in
//! `nerust_gui_shell::session::osd` and is shared with tao; this module
//! only renders into `GtkOverlay` children and honors wakeups via the
//! surface tick.

use std::{cell::RefCell, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use nerust_gui_shell::session::osd::{OsdManager, strip_pixels, strip_rows};
use nerust_render_traits::ReservedStrip;
use nerust_settings_core::model::local::{NotificationPosition, NotificationSettings};

use super::{State, renderer::GtkRenderer};

fn corner_align(position: NotificationPosition) -> (gtk::Align, gtk::Align) {
    match position {
        NotificationPosition::TopLeft => (gtk::Align::Start, gtk::Align::Start),
        NotificationPosition::TopRight => (gtk::Align::End, gtk::Align::Start),
        NotificationPosition::BottomLeft => (gtk::Align::Start, gtk::Align::End),
        NotificationPosition::BottomRight => (gtk::Align::End, gtk::Align::End),
    }
}

fn is_top(position: NotificationPosition) -> bool {
    matches!(
        position,
        NotificationPosition::TopLeft | NotificationPosition::TopRight
    )
}

pub(crate) struct GtkOsd {
    manager: OsdManager,
    transient_box: gtk::Box,
    status_label: gtk::Label,
    last_signature: String,
    last_strip: ReservedStrip,
}

impl GtkOsd {
    /// Wrap the window content in an overlay carrying OSD labels.
    /// Rendering stays native; labels composite like the menubar.
    pub(crate) fn attach(window: &gtk::ApplicationWindow) -> Self {
        let overlay = gtk::Overlay::new();
        let previous = window.child();
        if let Some(child) = previous.as_ref() {
            overlay.set_child(Some(child));
        }
        let transient_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        transient_box.set_can_target(false);
        transient_box.set_can_focus(false);
        let status_label = gtk::Label::new(None);
        status_label.set_can_target(false);
        status_label.set_can_focus(false);
        overlay.add_overlay(&transient_box);
        overlay.add_overlay(&status_label);
        overlay.set_clip_overlay(&transient_box, false);
        overlay.set_clip_overlay(&status_label, false);
        window.set_child(Some(&overlay));
        apply_osd_css();
        Self {
            manager: OsdManager::new(),
            transient_box,
            status_label,
            last_signature: String::new(),
            last_strip: ReservedStrip::default(),
        }
    }

    /// Drain the session queue, show modals, and refresh labels.
    /// Called from the surface tick (every frame while running).
    pub(crate) fn poll(
        &mut self,
        state: &Rc<RefCell<State>>,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
    ) {
        let now = Instant::now();
        let (drained, settings, language, metrics) = {
            let mut state = state.borrow_mut();
            let drained = state.drain_notifications();
            let snapshot = state.settings_snapshot().clone();
            let language = snapshot.shared.general.language;
            let (fps, paused) = state.emulation_status();
            (
                drained,
                snapshot.local.notifications.clone(),
                language,
                (fps, paused),
            )
        };
        if !drained.is_empty() {
            self.manager.push_all(drained, &settings, language, now);
        }
        self.manager.poll(now);
        let modal = self.manager.take_modal();
        if !modal.is_empty() {
            // Auto-pause first (spec): the game must not advance
            // behind the modal. Pause stays on; the user resumes.
            {
                use nerust_gui_shell::session::access::FrontendSession;
                state.borrow_mut().pause();
            }
            let detail = modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            show_error_modal(window, &detail);
        }
        let (fps, paused) = metrics;
        self.refresh_labels(&settings, language, paused, fps, window);
        self.apply_strip(&settings, renderer, window);
    }

    fn refresh_labels(
        &mut self,
        settings: &NotificationSettings,
        language: nerust_settings_core::model::language::AppLanguage,
        paused: bool,
        fps: f32,
        window: &gtk::ApplicationWindow,
    ) {
        let rows = self.manager.rows();
        let status = settings
            .status_enabled
            .then(|| OsdManager::status_text(fps, paused, language));
        let signature = format!(
            "{:?}|{:?}|{:?}|{:?}",
            rows.iter()
                .map(|row| (&row.text, row.rgb, row.glyph))
                .collect::<Vec<_>>(),
            status,
            settings.position,
            settings.status_position,
        );
        if signature == self.last_signature {
            return;
        }
        self.last_signature = signature;
        clear_box(&self.transient_box);
        let font_px = font_px_for(window);
        // Same-corner rule: status pins to the edge side, transient inside.
        let same_corner = settings.position == settings.status_position;
        let transient_top = is_top(settings.position);
        let mut top_rows: Vec<(char, String, (u8, u8, u8))> = Vec::new();
        let mut bottom_rows: Vec<(char, String, (u8, u8, u8))> = Vec::new();
        for row in &rows {
            let entry = (row.glyph, row.text.clone(), row.rgb);
            if transient_top {
                top_rows.push(entry);
            } else {
                bottom_rows.push(entry);
            }
        }
        if let Some(status) = status.as_ref() {
            let entry = ('●', status.clone(), (200, 200, 200));
            if same_corner {
                if transient_top {
                    top_rows.insert(0, entry);
                } else {
                    bottom_rows.push(entry);
                }
            } else if is_top(settings.status_position) {
                top_rows.insert(0, entry);
            } else {
                bottom_rows.push(entry);
            }
        }
        // Transient/status boxes anchor independently when split.
        if same_corner || transient_top == is_top(settings.status_position) {
            for (glyph, text, rgb) in top_rows.into_iter().chain(bottom_rows) {
                self.transient_box
                    .append(&osd_label(glyph, &text, rgb, font_px));
            }
            let (halign, valign) = corner_align(settings.position);
            self.transient_box.set_halign(halign);
            self.transient_box.set_valign(valign);
            self.status_label.set_visible(false);
        } else {
            for (glyph, text, rgb) in top_rows {
                self.transient_box
                    .append(&osd_label(glyph, &text, rgb, font_px));
            }
            let (halign, valign) = corner_align(settings.position);
            self.transient_box.set_halign(halign);
            self.transient_box.set_valign(valign);
            self.status_label.set_visible(true);
            if let Some(status) = status.as_ref() {
                self.status_label
                    .set_markup(&osd_markup('●', status, (200, 200, 200), font_px));
            }
            let (halign, valign) = corner_align(settings.status_position);
            self.status_label.set_halign(halign);
            self.status_label.set_valign(valign);
        }
        // Margins approximating the safe-area inset.
        for child in [
            self.transient_box.upcast_ref::<gtk::Widget>(),
            self.status_label.upcast_ref::<gtk::Widget>(),
        ] {
            child.set_margin_start(12);
            child.set_margin_end(12);
            child.set_margin_top(12);
            child.set_margin_bottom(12);
        }
    }

    fn apply_strip(
        &mut self,
        settings: &NotificationSettings,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
    ) {
        let client_h = window.height().max(0) as u32 * window.scale_factor().max(1) as u32;
        let (top_rows, bottom_rows) = strip_rows(
            settings.area_mode,
            settings.position,
            settings.status_position,
            settings.status_enabled,
        );
        let strip = ReservedStrip {
            top: strip_pixels(client_h, top_rows),
            bottom: strip_pixels(client_h, bottom_rows),
        };
        if strip != self.last_strip {
            self.last_strip = strip;
            renderer.borrow_mut().set_reserved_strip(strip);
        }
    }

    pub(crate) fn show_history(window: &gtk::ApplicationWindow, state: &Rc<RefCell<State>>) {
        let language = state.borrow().settings_snapshot().shared.general.language;
        let lines = state.borrow().notification_history_lines(language);
        let dialog = gtk::Dialog::with_buttons(
            Some(nerust_settings_core::i18n::text(
                language,
                nerust_settings_core::i18n::UiText::NotificationHistory,
            )),
            Some(window),
            gtk::DialogFlags::MODAL,
            &[("Close", gtk::ResponseType::Close)],
        );
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_min_content_height(300);
        scrolled.set_min_content_width(380);
        let buffer = gtk::TextBuffer::new(None);
        buffer.set_text(&lines.join("\n"));
        let view = gtk::TextView::with_buffer(&buffer);
        view.set_editable(false);
        view.set_cursor_visible(false);
        scrolled.set_child(Some(&view));
        dialog.content_area().append(&scrolled);
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }
}

fn font_px_for(window: &gtk::ApplicationWindow) -> f64 {
    // Viewport-relative sizing (2.2% of height), no floor: the OSD
    // scales with the game area like the tao twin.
    f64::from(window.height().max(1)) * 0.022
}

fn osd_markup(glyph: char, text: &str, rgb: (u8, u8, u8), font_px: f64) -> String {
    format!(
        "<span font_desc=\"Sans {:.1}\" foreground=\"#{:02X}{:02X}{:02X}\">{} {}</span>",
        font_px,
        rgb.0,
        rgb.1,
        rgb.2,
        glyph,
        glib::markup_escape_text(text)
    )
}

fn osd_label(glyph: char, text: &str, rgb: (u8, u8, u8), font_px: f64) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_markup(&osd_markup(glyph, text, rgb, font_px));
    label.set_xalign(0.0);
    label.set_can_target(false);
    label.set_can_focus(false);
    label.add_css_class("nerust-osd");
    label
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn apply_osd_css() {
    use std::sync::OnceLock;
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let css = gtk::CssProvider::new();
        css.load_from_data(
            ".nerust-osd { background-color: alpha(black, 0.72); border-radius: 6px; padding: 2px 6px; }",
        );
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("gdk display for OSD css"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

fn show_error_modal(parent: &gtk::ApplicationWindow, detail: &str) {
    // No libadwaita in tree: gtk MessageDialog twins tao's rfd modal.
    let dialog = gtk::MessageDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Error,
        gtk::ButtonsType::Ok,
        detail,
    );
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.present();
}

use gtk::gdk;
