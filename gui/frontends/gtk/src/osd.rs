//! GTK OSD: overlay labels, error modals, history.
//! Policy lives in shell `session::osd`; this renders into `GtkOverlay`.

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

/// Pill class for OSD labels. GTK4 exposes no background API
/// for labels, so one static rule is the only way to paint the
/// translucent pill; the provider installs once per process
/// (see `apply_osd_css`), keeping runtime cost at class match.
const OSD_PILL_CLASS: &str = "nerust-osd";
const OSD_PILL_CSS: &str =
    ".nerust-osd { background-color: alpha(black, 0.72); border-radius: 6px; padding: 2px 6px; }";

/// One OSD entry: text plus color (severity rides on the
/// `[INFO]`/`[WARN]`/`[ERROR]` tag prefix, not a badge).
type OsdEntry = (String, (u8, u8, u8));

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
        } else {
            // An overlay without a main child may leave overlay
            // children unallocated; a filling placeholder keeps
            // their allocation identical to the window.
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            placeholder.set_hexpand(true);
            placeholder.set_vexpand(true);
            overlay.set_child(Some(&placeholder));
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
    /// Takes `&mut State` directly: the tick already holds the
    /// borrow, so re-borrowing the `Rc` would panic.
    pub(crate) fn poll(
        &mut self,
        surface: &super::surface::Surface,
        state: &mut State,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
    ) {
        let now = Instant::now();
        let drained = state.drain_notifications();
        let snapshot = state.settings_snapshot().clone();
        let language = snapshot.shared.general.language;
        let metrics = state.emulation_status();
        let settings = snapshot.local.notifications.clone();
        if !drained.is_empty() {
            self.manager.push_all(drained, &settings, language, now);
        }
        let modal = self.manager.take_modal();
        // Auto-pause first, resume after the last ack.
        // Guarded by can_pause; only what we paused resumes.
        let mut resume_after_modal = false;
        if !modal.is_empty() {
            {
                use nerust_gui_shell::session::access::PauseControl;
                if state.can_pause() && !state.paused() {
                    state.pause();
                    resume_after_modal = true;
                }
            }
            let detail = modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.manager.queue_modal(detail);
        }
        // The batch take runs on the already-borrowed manager: the
        // Surface Rc is only touched inside the ack closure, which
        // runs after the tick borrow is long gone.
        if let Some(detail) = self.manager.take_modal_batch() {
            Self::present_modal(surface, window, detail, resume_after_modal);
        }
        let (fps, paused) = metrics;
        let changed = self.manager.poll(now);
        self.refresh_labels(&settings, language, paused, fps, window);
        self.apply_strip(&settings, renderer, window);
        // Repaint only while labels show (or just changed): the game
        // surface presents outside GTK paint and would otherwise cover
        // one-shot label paints within a frame.
        if changed || self.manager.has_visible() {
            self.redraw();
        }
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
                .map(|row| (&row.text, row.rgb, row.tag))
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
        let mut top_rows: Vec<OsdEntry> = Vec::new();
        let mut bottom_rows: Vec<OsdEntry> = Vec::new();
        for row in &rows {
            let entry = (format!("{} {}", row.tag, row.text), row.rgb);
            if transient_top {
                top_rows.push(entry);
            } else {
                bottom_rows.push(entry);
            }
        }
        if let Some(status) = status.as_ref() {
            let entry = (status.clone(), (200, 200, 200));
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
            for (text, rgb) in top_rows.into_iter().chain(bottom_rows) {
                self.transient_box.append(&osd_label(&text, rgb, font_px));
            }
            let (halign, valign) = corner_align(settings.position);
            self.transient_box.set_halign(halign);
            self.transient_box.set_valign(valign);
            self.status_label.set_visible(false);
        } else {
            for (text, rgb) in top_rows {
                self.transient_box.append(&osd_label(&text, rgb, font_px));
            }
            let (halign, valign) = corner_align(settings.position);
            self.transient_box.set_halign(halign);
            self.transient_box.set_valign(valign);
            self.status_label.set_visible(true);
            if let Some(status) = status.as_ref() {
                self.status_label
                    .set_markup(&osd_body_markup(status, (200, 200, 200), font_px));
            }
            let (halign, valign) = corner_align(settings.status_position);
            self.status_label.set_halign(halign);
            self.status_label.set_valign(valign);
        }
        // Margins matching the shared strip math (8px safe-area).
        for child in [
            self.transient_box.upcast_ref::<gtk::Widget>(),
            self.status_label.upcast_ref::<gtk::Widget>(),
        ] {
            child.set_margin_start(8);
            child.set_margin_end(8);
            child.set_margin_top(8);
            child.set_margin_bottom(8);
        }
    }

    /// Repaint labels over the freshly presented game frame. The
    /// game surface presents outside GTK paint, so one-shot label
    /// paints would be covered within a frame; repaint every tick.
    pub(crate) fn redraw(&self) {
        self.transient_box.queue_draw();
        self.status_label.queue_draw();
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
        use nerust_gui_shell::session::notification::NotificationLevel;
        let language = state.borrow().settings_snapshot().shared.general.language;
        let dialog = gtk::Dialog::with_buttons(
            Some(nerust_settings_core::i18n::text(
                language,
                nerust_settings_core::i18n::UiText::NotificationHistory,
            )),
            Some(window),
            gtk::DialogFlags::MODAL,
            &[(
                nerust_settings_core::i18n::text(
                    language,
                    nerust_settings_core::i18n::UiText::Close,
                ),
                gtk::ResponseType::Close,
            )],
        );
        // Toolbar (the Console / DevTools / logcat pattern: filter
        // plus clear live together at the top). 8 between controls.
        // Every caption goes through the i18n table (same keys as the
        // tao twin); level options reuse the settings level names.
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let level_caption = gtk::Label::new(Some(ui_text(language, UiText::HistoryLevel)));
        level_caption.set_xalign(0.0);
        let filter = gtk::ComboBoxText::new();
        for (id, label) in [
            ("all", ui_text(language, UiText::HistoryFilterAll)),
            ("info", ui_text(language, UiText::NotificationInfo)),
            ("warning", ui_text(language, UiText::NotificationWarning)),
            ("error", ui_text(language, UiText::Error)),
        ] {
            filter.append(Some(id), label);
        }
        filter.set_active_id(Some("all"));
        toolbar.append(&level_caption);
        toolbar.append(&filter);
        // "Clear History" names the target explicitly (cf. Event
        // Viewer "Clear Filter" vs "Clear Log"); count trails right.
        let clear = gtk::Button::with_label(ui_text(language, UiText::HistoryClear));
        toolbar.append(&clear);
        let count = gtk::Label::new(None);
        count.set_hexpand(true);
        count.set_xalign(1.0);
        toolbar.append(&count);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_min_content_height(300);
        scrolled.set_min_content_width(560);
        scrolled.set_vexpand(true);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(12);
        grid.set_row_spacing(2);
        scrolled.set_child(Some(&grid));
        // 16 content margin, 12 between groups.
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_start(16);
        body.set_margin_end(16);
        body.set_margin_top(16);
        body.set_margin_bottom(16);
        body.append(&toolbar);
        body.append(&scrolled);
        dialog.content_area().append(&body);
        let refresh = Rc::new({
            let state = Rc::clone(state);
            let grid = grid.clone();
            let filter = filter.clone();
            let count = count.clone();
            move || {
                let language = state.borrow().settings_snapshot().shared.general.language;
                let lines = state.borrow().notification_history_lines(language);
                let active = filter.active_id();
                while let Some(child) = grid.first_child() {
                    grid.remove(&child);
                }
                let visible: Vec<_> = lines
                    .iter()
                    .filter(|line| {
                        matches!(
                            (active.as_deref(), line.level),
                            (None, _)
                                | (Some("all"), _)
                                | (Some("info"), NotificationLevel::Info)
                                | (Some("warning"), NotificationLevel::Warning)
                                | (Some("error"), NotificationLevel::Error)
                        )
                    })
                    .collect();
                count.set_text(&nerust_settings_core::i18n::history_count(
                    language,
                    visible.len(),
                ));
                // Header: descriptive nouns, title case (Apple HIG).
                let mut row_idx = 0;
                for (col, header) in [
                    ui_text(language, UiText::HistoryDatetime),
                    ui_text(language, UiText::HistoryLevel),
                    ui_text(language, UiText::HistoryModule),
                    ui_text(language, UiText::HistoryMessage),
                ]
                .into_iter()
                .enumerate()
                {
                    let label = gtk::Label::new(Some(header));
                    label.set_xalign(0.0);
                    label.add_css_class("heading");
                    grid.attach(&label, col as i32, row_idx, 1, 1);
                }
                row_idx += 1;
                grid.attach(
                    &gtk::Separator::new(gtk::Orientation::Horizontal),
                    0,
                    row_idx,
                    4,
                    1,
                );
                row_idx += 1;
                if visible.is_empty() {
                    let empty = gtk::Label::new(Some(ui_text(language, UiText::HistoryEmpty)));
                    empty.set_xalign(0.0);
                    grid.attach(&empty, 0, row_idx, 4, 1);
                    return;
                }
                for line in visible {
                    let (r, g, b) = match line.level {
                        NotificationLevel::Info => (70u8, 70u8, 70u8),
                        NotificationLevel::Warning => (170u8, 110u8, 0u8),
                        NotificationLevel::Error => (190u8, 30u8, 30u8),
                    };
                    let mut text = line.text.clone();
                    if line.count > 1 {
                        text = format!("{text} ×{}", line.count);
                    }
                    let datetime = gtk::Label::new(None);
                    datetime.set_markup(&format!(
                        "<span font_family=\"monospace\" size=\"smaller\">{}</span>",
                        glib::markup_escape_text(&line.datetime),
                    ));
                    datetime.set_xalign(0.0);
                    let level = gtk::Label::new(None);
                    level.set_markup(&format!(
                        "<span foreground=\"#{r:02X}{g:02X}{b:02X}\">{}</span>",
                        nerust_gui_shell::session::osd::level_label(line.level, language),
                    ));
                    level.set_xalign(0.0);
                    let module = gtk::Label::new(None);
                    module.set_markup(&format!(
                        "<span font_family=\"monospace\" size=\"smaller\">{}</span>",
                        glib::markup_escape_text(line.module),
                    ));
                    module.set_xalign(0.0);
                    module.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    let message = gtk::Label::new(None);
                    message
                        .set_markup(&format!("<span>{}</span>", glib::markup_escape_text(&text),));
                    message.set_xalign(0.0);
                    message.set_wrap(true);
                    message.set_hexpand(true);
                    grid.attach(&datetime, 0, row_idx, 1, 1);
                    grid.attach(&level, 1, row_idx, 1, 1);
                    grid.attach(&module, 2, row_idx, 1, 1);
                    grid.attach(&message, 3, row_idx, 1, 1);
                    row_idx += 1;
                }
            }
        });
        refresh();
        filter.connect_changed({
            let refresh = Rc::clone(&refresh);
            move |_| refresh()
        });
        clear.connect_clicked({
            let state = Rc::clone(state);
            let refresh = Rc::clone(&refresh);
            move |_| {
                state.borrow_mut().clear_notification_history();
                refresh();
            }
        });
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }
}

fn font_px_for(window: &gtk::ApplicationWindow) -> f64 {
    // Viewport-relative sizing (2.2% of height), no floor: the OSD
    // scales with the game area like the tao twin.
    f64::from(window.height().max(1)) * 0.022
}

fn osd_body_markup(text: &str, rgb: (u8, u8, u8), font_px: f64) -> String {
    format!(
        "<span font_desc=\"Sans {:.1}\" foreground=\"#{:02X}{:02X}{:02X}\">{}</span>",
        font_px,
        rgb.0,
        rgb.1,
        rgb.2,
        glib::markup_escape_text(text)
    )
}

fn osd_label(text: &str, rgb: (u8, u8, u8), font_px: f64) -> gtk::Box {
    // Tagged pill: `[LEVEL] message` in one label, no badge chrome.
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let body = gtk::Label::new(None);
    body.set_markup(&osd_body_markup(text, rgb, font_px));
    body.set_xalign(0.0);
    body.set_can_target(false);
    body.set_can_focus(false);
    row.append(&body);
    row.add_css_class(OSD_PILL_CLASS);
    row.set_can_target(false);
    row.set_can_focus(false);
    row
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
        let Some(display) = gdk::Display::default() else {
            log::warn!("OSD pill style skipped: no gdk display");
            return;
        };
        let css = gtk::CssProvider::new();
        css.load_from_data(OSD_PILL_CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

impl GtkOsd {
    /// Present one error batch; the ack chains the next.
    /// Resume only after the last batch iff auto-paused.
    fn present_modal(
        surface: &super::surface::Surface,
        window: &gtk::ApplicationWindow,
        detail: String,
        resume_after_modal: bool,
    ) {
        let dialog = gtk::MessageDialog::new(
            Some(window),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Ok,
            &detail,
        );
        let surface = surface.clone();
        let window = window.clone();
        dialog.connect_response(move |dialog, _| {
            dialog.close();
            let next = {
                let surface = surface.borrow_mut();
                let mut osd = surface.osd.borrow_mut();
                osd.manager.modal_closed();
                osd.manager.take_modal_batch()
            };
            if let Some(next) = next {
                Self::present_modal(&surface, &window, next, resume_after_modal);
            } else if resume_after_modal {
                use nerust_gui_shell::session::access::PauseControl;
                surface.borrow().state.borrow_mut().resume();
            }
        });
        dialog.present();
    }
}

use gtk::gdk;
