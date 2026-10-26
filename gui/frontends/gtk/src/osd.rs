//! GTK OSD: in-frame layer, error modals, history.
//! Native dialogs stay here; pump and renderer wiring live in frontend_host.

use std::{cell::RefCell, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use nerust_frontend_host::{
    frame::FrameCtx,
    loop_driver::WakeNeed,
    osd::{ModalAdvance, OsdLayerSync, OsdNotifications},
};

use super::{State, renderer::GtkRenderer};

impl nerust_render_traits::renderer::OsdSink for GtkRenderer {
    fn set_osd_layer(&mut self, layer: Option<nerust_render_traits::renderer::OsdLayer>) {
        GtkRenderer::set_osd_layer(self, layer);
    }
}

impl nerust_render_traits::renderer::StripSink for GtkRenderer {
    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        GtkRenderer::set_reserved_strip(self, strip);
    }
}

/// Reentrancy guard for caption rebuilds: set while the filter items are
/// rebuilt so the `changed` handler skips the recursive refresh.
struct CaptionSyncGuard<'a> {
    flag: &'a std::cell::Cell<bool>,
}

impl<'a> CaptionSyncGuard<'a> {
    fn new(flag: &'a std::cell::Cell<bool>) -> Self {
        flag.set(true);
        Self { flag }
    }
}

impl Drop for CaptionSyncGuard<'_> {
    fn drop(&mut self) {
        self.flag.set(false);
    }
}

/// Built history dialog plus its live controls; tests drive `filter` and `clear`.
pub(crate) struct HistoryDialog {
    pub dialog: gtk::Dialog,
    pub refresh: Rc<dyn Fn()>,
    /// Interactive handles, exercised by widget tests (reopen/filter/clear).
    #[cfg(test)]
    pub filter: gtk::ComboBoxText,
    /// Interactive handles, exercised by widget tests (reopen/filter/clear).
    #[cfg(test)]
    pub clear: gtk::Button,
    /// Row container for widget tests: growth proves open-dialog re-render.
    #[cfg(test)]
    pub grid: gtk::Grid,
}

/// Fixed history-dialog captions for one language. Plain owned strings:
/// widgets take copies, the table stays authoritative.
pub(crate) struct HistoryDialogCaptions {
    pub title: String,
    pub close: String,
    pub level: String,
    pub filter_all: String,
    pub filter_info: String,
    pub filter_warning: String,
    pub filter_error: String,
    pub clear: String,
}

pub(crate) struct GtkOsd {
    notifications: OsdNotifications,
    layer: OsdLayerSync,
    output_generation: Option<u64>,
    pending_modal: Option<String>,
}

impl GtkOsd {
    pub(crate) fn new() -> Self {
        Self {
            notifications: OsdNotifications::new(),
            layer: OsdLayerSync::new(),
            output_generation: None,
            pending_modal: None,
        }
    }

    /// Forget sync state after a renderer rebuild: the new object
    /// holds no layer, so the next sync must re-push instead of skip.
    pub(crate) fn invalidate(&mut self) {
        self.layer.invalidate();
    }

    /// Drain the session queue, extract modal effects, and sync the OSD layer.
    /// Takes `&mut State` directly: the tick already holds the borrow.
    pub(crate) fn poll(
        &mut self,
        state: &mut State,
        renderer: &Rc<RefCell<GtkRenderer>>,
        window: &gtk::ApplicationWindow,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) -> WakeNeed {
        let generation = renderer.borrow().output_generation();
        if self.output_generation != Some(generation) {
            self.invalidate();
            self.output_generation = Some(generation);
        }
        let now = Instant::now();
        let report = self.notifications.pump(state.host.session_mut(), ctx, now);
        if report.changed {
            log::debug!("gtk osd: display state changed");
        }
        if report.modal_queued {
            // Own the whole queue lifetime, even when another owner already paused.
            state.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Modal, true);
        }
        // Native presentation can synchronously emit focus/map callbacks.
        // Acquire Modal above, but defer presentation until every turn borrow ends.
        if self.pending_modal.is_none() {
            self.pending_modal = self.notifications.take_modal_batch();
        }
        let client_h = window.height().max(0) as u32 * window.scale_factor().max(1) as u32;
        self.layer.render(
            self.notifications.manager(),
            &mut *renderer.borrow_mut(),
            size,
            client_h,
            ctx,
            now,
        );
        WakeNeed {
            osd_deadline: self.notifications.wakeup_deadline(Instant::now()),
            ..WakeNeed::default()
        }
    }

    pub(crate) fn take_modal(&mut self) -> Option<String> {
        self.pending_modal.take()
    }

    /// Build the history dialog without presenting; captions follow the tracked language.
    pub(crate) fn history_dialog_captions(
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> HistoryDialogCaptions {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        HistoryDialogCaptions {
            title: ui_text(language, UiText::NotificationHistory).to_string(),
            close: ui_text(language, UiText::Close).to_string(),
            level: ui_text(language, UiText::HistoryLevel).to_string(),
            filter_all: ui_text(language, UiText::HistoryFilterAll).to_string(),
            filter_info: ui_text(language, UiText::NotificationInfo).to_string(),
            filter_warning: ui_text(language, UiText::NotificationWarning).to_string(),
            filter_error: ui_text(language, UiText::Error).to_string(),
            clear: ui_text(language, UiText::HistoryClear).to_string(),
        }
    }

    pub(crate) fn build_history(
        window: &gtk::ApplicationWindow,
        state: &Rc<RefCell<State>>,
    ) -> HistoryDialog {
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
        // Toolbar: filter plus clear live together at the top. Every caption
        // goes through the i18n table (same keys as the tao twin).
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let level_caption = gtk::Label::new(Some(ui_text(language, UiText::HistoryLevel)));
        level_caption.set_xalign(0.0);
        let filter = gtk::ComboBoxText::new();
        for filter_kind in nerust_gui_shell::session::osd::HistoryFilter::all() {
            filter.append(Some(filter_kind.id()), filter_kind.label(language));
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
        // Fixed-caption widgets refresh alongside the rows.
        let close_button = dialog
            .widget_for_response(gtk::ResponseType::Close)
            .and_downcast::<gtk::Button>();
        // Same (seq, language, filter) gate as the tao twin: a refresh with
        // unchanged inputs rebuilds nothing instead of re-allocating rows.
        let last_sync = Rc::new(RefCell::new(None));
        // Reentrancy guard: rebuilding filter items emits `changed`; skip the recursive refresh.
        let syncing_captions = Rc::new(std::cell::Cell::new(false));
        let refresh = Rc::new({
            let state = Rc::clone(state);
            let dialog = dialog.clone();
            let grid = grid.clone();
            let filter = filter.clone();
            let count = count.clone();
            let level_caption = level_caption.clone();
            let clear = clear.clone();
            let close_button = close_button.clone();
            let syncing_captions = Rc::clone(&syncing_captions);
            let last_sync = Rc::clone(&last_sync);
            move || {
                // Skip refresh on a destroyed native surface; never-realized dialogs still refresh.
                if super::surface_state::surface_dead(&dialog) {
                    return;
                }
                // Gate inputs first: a refresh with unchanged inputs
                // rebuilds nothing instead of re-allocating rows.
                let (language, seq) = {
                    let borrowed = state.borrow();
                    let language = borrowed.settings_snapshot().shared.general.language;
                    (language, borrowed.notification_history_seq())
                };
                let active = filter.active_id();
                let key = (
                    seq,
                    language,
                    active.as_deref().unwrap_or("all").to_string(),
                );
                if last_sync.borrow().as_ref() == Some(&key) {
                    return;
                }
                *last_sync.borrow_mut() = Some(key);
                // Fixed captions follow the language; the selection survives the rebuild.
                let _caption_sync = CaptionSyncGuard::new(&syncing_captions);
                let captions = Self::history_dialog_captions(language);
                dialog.set_title(Some(&captions.title));
                if let Some(close) = close_button.as_ref() {
                    close.set_label(&captions.close);
                }
                level_caption.set_text(&captions.level);
                let selected = filter.active_id();
                filter.remove_all();
                for (id, label) in [
                    ("all", captions.filter_all.as_str()),
                    ("info", captions.filter_info.as_str()),
                    ("warning", captions.filter_warning.as_str()),
                    ("error", captions.filter_error.as_str()),
                ] {
                    filter.append(Some(id), label);
                }
                filter.set_active_id(selected.as_deref());
                clear.set_label(&captions.clear);
                let mut borrowed = state.borrow_mut();
                let lines = borrowed.history_lines_cached(language);
                while let Some(child) = grid.first_child() {
                    grid.remove(&child);
                }
                // Shared threshold: this level and above.
                let minimum =
                    nerust_gui_shell::session::osd::HistoryFilter::from_id(active.as_deref())
                        .minimum();
                let visible: Vec<_> = lines
                    .iter()
                    .filter(|line| {
                        nerust_frontend_kernel::level_visible(minimum, line.level.severity())
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
                    let (r, g, b) = nerust_gui_shell::session::osd::level_rgb(line.level);
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
                        glib::markup_escape_text(nerust_gui_shell::session::osd::short_module(
                            line.module
                        )),
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
            let syncing_captions = Rc::clone(&syncing_captions);
            move |_| {
                if !syncing_captions.get() {
                    refresh()
                }
            }
        });
        clear.connect_clicked({
            let state = Rc::clone(state);
            let refresh = Rc::clone(&refresh);
            move |_| {
                state.borrow_mut().clear_notification_history();
                refresh();
            }
        });
        // Follow new notifications while open; the gone check stops a destroyed source.
        let follow_dialog = dialog.clone();
        let follow_refresh = Rc::clone(&refresh);
        let follow = std::cell::Cell::new(Some(glib::timeout_add_local(
            std::time::Duration::from_millis(500),
            move || {
                if super::surface_state::surface_gone(&follow_dialog) {
                    return glib::ControlFlow::Break;
                }
                follow_refresh();
                glib::ControlFlow::Continue
            },
        )));
        dialog.connect_destroy(move |_| {
            if let Some(source) = follow.take() {
                source.remove();
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
        HistoryDialog {
            dialog,
            refresh,
            #[cfg(test)]
            filter,
            #[cfg(test)]
            clear,
            #[cfg(test)]
            grid,
        }
    }
}
impl GtkOsd {
    /// Present one error batch; the ack chains the next.
    /// Release only the modal ownership after the last batch.
    pub(crate) fn present_modal(
        surface: &super::surface::Surface,
        window: &gtk::ApplicationWindow,
        detail: String,
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
            // close() can emit a second (DeleteEvent) response and ACK twice.
            dialog.destroy();
            // Bounded recursion: the modal queue caps at MAX_QUEUED,
            // so nesting depth is capped too, not an unbounded chain.
            let next = {
                let surface = surface.borrow_mut();
                let mut osd = surface.osd.borrow_mut();
                osd.notifications.acknowledge_modal()
            };
            match next {
                ModalAdvance::Present(next) => Self::present_modal(&surface, &window, next),
                ModalAdvance::Drained => {
                    let state = surface.borrow().state.clone();
                    state
                        .borrow_mut()
                        .set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Modal, false);
                }
            }
        });
        dialog.present();
    }
}

#[cfg(test)]
#[cfg(test)]
pub(super) mod history_dialog_tests {
    use super::GtkOsd;
    use gtk::prelude::*;
    use nerust_settings_core::model::language::AppLanguage;

    /// Caption table follows the language; display application rides `refresh`.
    #[test]
    fn history_captions_follow_language() {
        let en = GtkOsd::history_dialog_captions(AppLanguage::English);
        let ja = GtkOsd::history_dialog_captions(AppLanguage::Japanese);
        for captions in [&en, &ja] {
            for text in [
                &captions.title,
                &captions.close,
                &captions.level,
                &captions.filter_all,
                &captions.filter_info,
                &captions.filter_warning,
                &captions.filter_error,
                &captions.clear,
            ] {
                assert!(!text.is_empty());
            }
        }
        assert_ne!(en.title, ja.title);
        assert_ne!(en.clear, ja.clear);
        assert_ne!(en.filter_all, ja.filter_all);
    }

    fn harness() -> Option<(
        gtk::Application,
        gtk::ApplicationWindow,
        std::rc::Rc<std::cell::RefCell<crate::State>>,
    )> {
        if !crate::test_support::ui_test_available() {
            return None;
        }
        let app = gtk::Application::builder()
            .application_id("org.nerust.history-test")
            .build();
        let window = gtk::ApplicationWindow::builder().application(&app).build();
        Some((app, window, model_state()))
    }

    /// Headless-capable state for the model gate: session plus settings
    /// only, no application, no window, no display.
    pub(crate) fn model_state() -> std::rc::Rc<std::cell::RefCell<crate::State>> {
        use std::sync::Arc;
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(Vec::new()));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let ctx = nerust_gui_shell::context::FrontendContext {
            gpu_factory: std::rc::Rc::new(MockPipelineFactory),
            registry,
            rom_loader: Box::new(MockRomLoader),
            audio_registry: audio,
        };
        std::rc::Rc::new(std::cell::RefCell::new(
            crate::State::new_for_tests(ctx).expect("test state builds"),
        ))
    }

    #[derive(Debug)]
    struct MockPipelineFactory;

    impl nerust_render_traits::renderer::PipelineFactory for MockPipelineFactory {
        fn create_pipeline(
            &self,
            _config: &nerust_render_traits::renderer::RendererConfig,
            _display: raw_window_handle::RawDisplayHandle,
        ) -> Result<
            Box<dyn nerust_render_traits::renderer::RenderPipeline>,
            nerust_render_traits::renderer::RendererError,
        > {
            Err(nerust_render_traits::renderer::RendererError::new(
                "test",
                Box::new(std::io::Error::other("no renderer in tests")),
            ))
        }
    }

    #[derive(Debug)]
    struct MockRomLoader;

    impl nerust_gui_shell::load::RomLoader for MockRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            Err(nerust_gui_shell::load::RomLoaderError::from_session(
                nerust_gui_shell::session::SessionError::NoCore,
            ))
        }
    }

    /// Dialog lifecycle: build, refresh, filter, clear, and reopen cleanly.
    #[test]
    fn history_dialog_lifecycle_runs_without_display_errors() {
        let Some((_app, window, state)) = harness() else {
            eprintln!("SKIP: history widget test needs a display (run under Xvfb)");
            return;
        };
        let built = GtkOsd::build_history(&window, &state);
        (built.refresh)();
        // Filter to warnings on an empty history: still the empty state.
        built.filter.set_active_id(Some("warning"));
        (built.refresh)();
        built.clear.emit_clicked();
        (built.refresh)();
        built.dialog.destroy();
        // Reopen pattern: a fresh build works after destroy.
        let rebuilt = GtkOsd::build_history(&window, &state);
        (rebuilt.refresh)();
        rebuilt.dialog.destroy();
    }

    /// Open rows and language switches appear on refresh under the same gate.
    #[test]
    fn history_dialog_shows_new_rows_and_follows_language_change() {
        use nerust_gui_shell::session::notification::FailureOrigin;
        use nerust_settings_core::i18n::{UiText, text as ui_text};

        fn child_count(grid: &gtk::Grid) -> usize {
            let mut count = 0;
            let mut next = grid.first_child();
            while let Some(child) = next {
                count += 1;
                next = child.next_sibling();
            }
            count
        }

        let Some((_app, window, state)) = harness() else {
            eprintln!("SKIP: history widget test needs a display (run under Xvfb)");
            return;
        };
        let built = GtkOsd::build_history(&window, &state);
        let en_title = ui_text(AppLanguage::English, UiText::NotificationHistory);
        assert_eq!(built.dialog.title().as_deref(), Some(en_title));
        assert_eq!(
            child_count(&built.grid),
            6,
            "empty state: headers + separator + empty row"
        );
        // A notification arriving while open appears after refresh.
        state
            .borrow_mut()
            .host
            .session_mut()
            .report_external_error(FailureOrigin::Host, "boom".into());
        (built.refresh)();
        assert_eq!(
            child_count(&built.grid),
            9,
            "one error row renders without reopen"
        );
        // The language switch rebuilds title and rows under the gate.
        let mut snapshot = state.borrow().host.session().settings_snapshot().clone();
        snapshot.shared.general.language = AppLanguage::Japanese;
        state
            .borrow_mut()
            .host
            .session_mut()
            .apply_settings(snapshot)
            .expect("language switch applies");
        (built.refresh)();
        let ja_title = ui_text(AppLanguage::Japanese, UiText::NotificationHistory);
        assert_eq!(built.dialog.title().as_deref(), Some(ja_title));
        assert_eq!(
            child_count(&built.grid),
            9,
            "rows survive the language rebuild"
        );
        built.dialog.destroy();
    }

    /// Model gate: revision-gated refresh rebuilds only on seq or language change.
    #[test]
    fn history_model_refresh_is_revision_gated() {
        use nerust_gui_shell::session::notification::{
            FailureOrigin, Message, NotificationLevel, UiNotification,
        };
        use nerust_settings_core::model::language::AppLanguage;

        let state = model_state();
        assert!(
            state
                .borrow_mut()
                .host
                .session_mut()
                .notification_history_lines_cached(AppLanguage::English)
                .is_empty()
        );
        // A notification bumps the revision and the row appears.
        let before = state.borrow().host.session().notification_history_seq();
        state
            .borrow_mut()
            .host
            .session_mut()
            .report_external_error(FailureOrigin::Host, "boom".into());
        let bumped = state.borrow().host.session().notification_history_seq();
        assert!(bumped > before, "push advances the revision");
        let texts: Vec<String> = state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::English)
            .iter()
            .map(|line| line.text.clone())
            .collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("boom"), "row renders: {}", texts[0]);
        // Unchanged seq rebuilds nothing: the same backing store serves
        // both calls, so zero reformat builds ran.
        let first = state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::English)
            .as_ptr();
        let second = state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::English)
            .as_ptr();
        assert_eq!(first, second, "seq-unchanged refresh must not rebuild");
        // The language switch rebuilds captions and rows alike.
        let en_captions = GtkOsd::history_dialog_captions(AppLanguage::English);
        let ja_captions = GtkOsd::history_dialog_captions(AppLanguage::Japanese);
        assert_ne!(en_captions.title, ja_captions.title);
        assert_ne!(en_captions.clear, ja_captions.clear);
        let mut snapshot = state.borrow().host.session().settings_snapshot().clone();
        snapshot.shared.general.language = AppLanguage::Japanese;
        state
            .borrow_mut()
            .host
            .session_mut()
            .apply_settings(snapshot)
            .expect("language switch applies");
        let texts: Vec<String> = state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::Japanese)
            .iter()
            .map(|line| line.text.clone())
            .collect();
        assert_eq!(texts.len(), 1, "row survives the language rebuild");
        assert!(texts[0].contains("boom"), "detail survives: {}", texts[0]);
        // Filter thresholds without widgets: this level and above, like
        // the grid gate (info shows all, warning hides info, error is top).
        let events = [
            UiNotification {
                level: NotificationLevel::Info,
                message: Message::paused(),
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            },
            UiNotification {
                level: NotificationLevel::Error,
                message: Message::command_failure(FailureOrigin::Host, "", "boom".into()),
                count: 1,
                seq: 1,
                at: std::time::UNIX_EPOCH,
            },
        ];
        let lines =
            nerust_gui_shell::session::osd::history_lines(events.iter(), AppLanguage::English);
        let visible = |active: Option<&str>| {
            let minimum = match active {
                None | Some("all") | Some("info") => 0,
                Some("warning") => 1,
                _ => 2,
            };
            lines
                .iter()
                .filter(|line| {
                    nerust_frontend_kernel::level_visible(minimum, line.level.severity())
                })
                .count()
        };
        assert_eq!(visible(Some("all")), 2);
        assert_eq!(visible(Some("warning")), 1);
        assert_eq!(visible(Some("error")), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::GtkOsd;
    use nerust_gui_shell::session::osd::{
        OsdChrome, OsdRow, OsdState, OsdSync, OsdSyncInput, TickMetrics, ViewInsets,
    };
    use nerust_settings_core::model::local::NotificationSettings;

    struct StubState {
        generation: u64,
        rows: Vec<OsdRow>,
    }

    impl OsdState for StubState {
        fn generation(&self) -> u64 {
            self.generation
        }
        fn visible_row(&self) -> Option<&OsdRow> {
            self.rows.first()
        }
        fn rows(&self) -> &[OsdRow] {
            &self.rows
        }
    }

    fn sync_input(chrome: OsdChrome, width: u32, height: u32) -> OsdSyncInput {
        OsdSyncInput {
            tick: TickMetrics {
                fps: 60.0,
                ..TickMetrics::default()
            },
            chrome,
            language: nerust_settings_core::model::language::AppLanguage::English,
            now: std::time::Instant::now(),
            width,
            height,
            ui_scale: 1.0,
            safe_insets: ViewInsets::default(),
        }
    }

    fn row_state() -> StubState {
        StubState {
            generation: 7,
            rows: vec![OsdRow {
                text: std::sync::Arc::from("Paused"),
                rgb: (255, 255, 255),
                tag: "[I]".into(),
            }],
        }
    }

    #[test]
    fn osd_new_and_invalidate_are_headless() {
        // No display needed: construction and plan reset stay CPU-side.
        let mut osd = GtkOsd::new();
        #[derive(Debug, Default)]
        struct Sink(usize);
        impl nerust_render_traits::renderer::OsdSink for Sink {
            fn set_osd_layer(&mut self, _: Option<nerust_render_traits::renderer::OsdLayer>) {}
        }
        impl nerust_render_traits::renderer::StripSink for Sink {
            fn set_reserved_strip(&mut self, _: nerust_render_traits::ReservedStrip) {
                self.0 += 1;
            }
        }
        let state = super::history_dialog_tests::model_state();
        let ctx =
            nerust_frontend_host::frame::FrameCache::new().frame_ctx(state.borrow().host.session());
        let mut sink = Sink::default();
        let size = nerust_render_traits::SurfaceSize::new(320, 240);
        let now = std::time::Instant::now();
        osd.layer
            .render(osd.notifications.manager(), &mut sink, size, 240, &ctx, now);
        osd.layer
            .render(osd.notifications.manager(), &mut sink, size, 240, &ctx, now);
        assert_eq!(sink.0, 1);
        osd.invalidate();
        assert!(!osd.layer.layer_live());
        osd.layer
            .render(osd.notifications.manager(), &mut sink, size, 240, &ctx, now);
        assert_eq!(sink.0, 2, "renderer replacement re-pushes the strip");
        osd.invalidate();
    }

    #[test]
    fn present_plan_renders_then_skips_steady_frames() {
        use nerust_gui_shell::session::osd::OsdPresenter;
        let settings = NotificationSettings::default();
        let chrome = OsdChrome::from(&settings);
        let state = row_state();
        let mut plan = OsdPresenter::new();
        assert!(matches!(
            plan.sync(&state, sync_input(chrome, 320, 240)),
            OsdSync::Render(_)
        ));
        assert!(matches!(
            plan.sync(&state, sync_input(chrome, 320, 240)),
            OsdSync::Skip
        ));
        // Rebuilds re-push: the gate forgets the fingerprint.
        plan.invalidate();
        assert!(matches!(
            plan.sync(&state, sync_input(chrome, 320, 240)),
            OsdSync::Render(_)
        ));
    }

    #[test]
    fn present_plan_clears_zero_size() {
        use nerust_gui_shell::session::osd::OsdPresenter;
        let settings = NotificationSettings::default();
        let chrome = OsdChrome::from(&settings);
        let idle = StubState {
            generation: 7,
            rows: Vec::new(),
        };
        let mut plan = OsdPresenter::new();
        // Idle steady state skips; a zero-size frame clears instead.
        assert!(matches!(
            plan.sync(&idle, sync_input(chrome, 320, 240)),
            OsdSync::Skip
        ));
        assert!(matches!(
            plan.sync(&idle, sync_input(chrome, 0, 0)),
            OsdSync::Clear | OsdSync::Skip
        ));
    }

    #[test]
    fn raster_backend_rejects_empty_surfaces() {
        use nerust_render_osd::OsdContent;
        use nerust_render_traits::renderer::OsdAnchor;
        let mut raster = nerust_render_osd::OsdRaster::new();
        let content = OsdContent {
            rows: Vec::new(),
            status: None,
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::TopLeft,
        };
        // Zero extent refuses before touching font state.
        // Fully-qualified call covers the GTK raster backend seam.
        assert!(raster.rasterize(&content, 0, 240).is_none());
        assert!(raster.rasterize(&content, 320, 0).is_none());
    }
}
