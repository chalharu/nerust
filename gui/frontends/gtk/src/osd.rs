//! GTK OSD: in-frame layer, error modals, history.
//! Policy lives in shell `session::osd`; rows rasterize to a layer
//! the renderer composites, so no overlay widgets are needed.

use std::{cell::RefCell, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use nerust_gui_shell::session::osd::{ModalSource, OsdManager};
use nerust_settings_core::model::local::NotificationSettings;

use super::{State, renderer::GtkRenderer};

pub(crate) struct GtkOsd {
    manager: OsdManager,
    raster: nerust_render_osd::OsdRaster,
    last_strip: nerust_render_traits::ReservedStrip,
    /// Owns the whole sync fingerprint (dirty flag, status gate,
    /// surface size, layer liveness). Shared decision procedure
    /// with the tao frontend; this struct only rasterizes.
    presenter: nerust_gui_shell::session::osd::OsdPresenter,
}

impl GtkOsd {
    pub(crate) fn new() -> Self {
        Self {
            manager: OsdManager::new(),
            raster: nerust_render_osd::OsdRaster::new(),
            last_strip: nerust_render_traits::ReservedStrip::default(),
            presenter: nerust_gui_shell::session::osd::OsdPresenter::new(),
        }
    }

    /// Forget sync state after a renderer rebuild: the new object
    /// holds no layer, so the next sync must re-push instead of skip.
    pub(crate) fn invalidate(&mut self) {
        self.presenter.invalidate();
    }

    /// Drain the session queue, show modals, and sync the OSD layer.
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
        use nerust_gui_shell::session::osd::{PushInput, TransientPolicy};
        let now = Instant::now();
        let language = state.settings_snapshot().shared.general.language;
        let tick = state.session.tick_metrics();
        let settings = state.settings_snapshot().local.notifications.clone();
        let report = state.session.pump_osd(
            &mut self.manager,
            PushInput {
                policy: TransientPolicy::from(&settings),
                language,
                now,
                tick,
            },
        );
        if report.changed {
            log::debug!("gtk osd: display state changed");
        }
        let mut resume_after_modal = false;
        if report.modal_queued {
            // Auto-pause first, guarded by can_pause.
            // Only what we paused resumes, after the last ack.
            {
                use nerust_gui_shell::session::access::PauseControl;
                if tick.can_pause() && !tick.paused {
                    state.pause();
                    resume_after_modal = true;
                }
            }
        }
        // The batch take runs on the already-borrowed manager: the
        // Surface Rc is only touched inside the ack closure, which
        // runs after the tick borrow is long gone.
        if let Some(detail) = self.manager.take_modal_batch() {
            Self::present_modal(surface, window, detail, resume_after_modal);
        }
        self.apply_strip(&settings, renderer, window);
        self.sync_layer(&settings, language, tick.paused, tick.fps, renderer, window);
    }

    /// Reserve edge strips from settings. Game viewport fits into
    /// the remaining rect; strips stay clear for the OSD layer.
    fn apply_strip(
        &mut self,
        settings: &NotificationSettings,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
    ) {
        use nerust_gui_shell::session::osd::strip_rows;
        use nerust_render_traits::ReservedStrip;
        use nerust_render_traits::renderer::strip_pixels;
        let client_h = window.height().max(0) as u32 * window.scale_factor().max(1) as u32;
        let (top_rows, bottom_rows) = strip_rows(
            settings.area_mode,
            settings.position,
            settings.status_area_mode,
            settings.status_position,
            settings.status_enabled,
        );
        let strip = ReservedStrip {
            top: strip_pixels(client_h, top_rows, 1.0),
            bottom: strip_pixels(client_h, bottom_rows, 1.0),
        };
        if strip != self.last_strip {
            self.last_strip = strip;
            renderer.borrow_mut().set_reserved_strip(strip);
        }
    }

    /// Rasterize on change and hand the layer to the renderer.
    /// The sync decision (dirty flag, status gate, size, liveness)
    /// lives in shell `OsdPresenter`; this only rasterizes plans.
    fn sync_layer(
        &mut self,
        settings: &NotificationSettings,
        language: nerust_settings_core::model::language::AppLanguage,
        paused: bool,
        fps: f32,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
    ) {
        use nerust_gui_shell::session::osd::{OsdSync, OsdSyncInput};
        let scale = window.scale_factor().max(1) as u32;
        let w = window.width().max(0) as u32 * scale;
        let h = window.height().max(0) as u32 * scale;
        let chrome = nerust_gui_shell::session::osd::OsdChrome::from(settings);
        let decision = self.presenter.sync(
            &self.manager,
            OsdSyncInput {
                chrome,
                language,
                fps,
                paused,
                now: Instant::now(),
                width: w,
                height: h,
                ui_scale: 1.0,
                safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
            },
        );
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => renderer.borrow_mut().set_osd_layer(None),
            OsdSync::Render(plan) => {
                let (width, height) = (plan.width, plan.height);
                let content = plan.into_content();
                let layer = self.raster.rasterize(&content, width, height);
                renderer.borrow_mut().set_osd_layer(layer);
            }
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
            gtk::DialogFlags::DESTROY_WITH_PARENT,
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
        // Same (seq, language, filter) gate as the tao twin: a refresh
        // with unchanged inputs rebuilds nothing instead of
        // re-allocating every row and label.
        let last_sync = Rc::new(RefCell::new(None));
        let refresh = Rc::new({
            let state = Rc::clone(state);
            let grid = grid.clone();
            let filter = filter.clone();
            let count = count.clone();
            let last_sync = Rc::clone(&last_sync);
            move || {
                let language = state.borrow().settings_snapshot().shared.general.language;
                let active = filter.active_id();
                let key = (
                    state.borrow().notification_history_seq(),
                    language,
                    active.as_deref().unwrap_or("all").to_string(),
                );
                if last_sync.borrow().as_ref() == Some(&key) {
                    return;
                }
                *last_sync.borrow_mut() = Some(key);
                let lines = state.borrow().notification_history_lines(language);
                while let Some(child) = grid.first_child() {
                    grid.remove(&child);
                }
                // Threshold match like the tao twin: this level and
                // above, so Info shows all (no lower level exists).
                let minimum = match active.as_deref() {
                    None | Some("all") | Some("info") => 0,
                    Some("warning") => 1,
                    _ => 2,
                };
                let visible: Vec<_> = lines
                    .iter()
                    .filter(|line| line.level.severity() >= minimum)
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
                    // `HistoryLine.text` already carries the `×N` suffix
                    // when merged; use it as-is.
                    let text = line.text.clone();
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
        // Modeless like the tao twin: a modal would park all keys
        // in the dialog. Destroy (not hide) so focus returns.
        if let Some(close) = dialog.widget_for_response(gtk::ResponseType::Close) {
            close.set_margin_start(16);
            close.set_margin_end(16);
            close.set_margin_bottom(16);
        }
        dialog.connect_response(|dialog, _| dialog.destroy());
        dialog.present();
    }
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
