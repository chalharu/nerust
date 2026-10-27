//! Grouped native notification history controls and reentrant model delivery.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};
use nerust_frontend_host::{
    history::{HistoryController, HistoryModel, HistoryVisitor},
    window::Window as HostWindow,
};

use crate::State;
use nerust_gui_shell::session::osd::{HistoryFilter, HistoryRowView};

/// Reentrancy guard for caption rebuilds: set while the filter items are
/// rebuilt so the `changed` handler skips the recursive refresh.
struct CaptionSyncGuard<'a> {
    flag: &'a std::cell::Cell<bool>,
    previous: bool,
}

impl<'a> CaptionSyncGuard<'a> {
    fn new(flag: &'a std::cell::Cell<bool>) -> Self {
        Self {
            flag,
            previous: flag.replace(true),
        }
    }
}

impl Drop for CaptionSyncGuard<'_> {
    fn drop(&mut self) {
        self.flag.set(self.previous);
    }
}

/// A reentrant native setter may submit a newer owned model. Finish the outer
/// traversal, then replay the latest one without holding any mailbox borrow.
#[derive(Default)]
struct HistoryDelivery {
    active: Cell<bool>,
    pending: RefCell<Option<HistoryModel>>,
}

impl HistoryDelivery {
    fn submit(&self, model: HistoryModel, visitor: &mut GtkHistoryVisitor) {
        *self.pending.borrow_mut() = Some(model);
        if self.active.get() {
            return;
        }
        let _active = CaptionSyncGuard::new(&self.active);
        loop {
            let next = self.pending.borrow_mut().take();
            let Some(model) = next else { break };
            // HistoryModel owns the localized captions; project its title onto
            // the common window before native chrome and grouped body dispatch.
            let mut window = visitor.window.clone();
            window.title.clone_from(&model.captions.title);
            window.visit(&mut crate::window::GtkWindowRenderer::History(
                &model, visitor,
            ));
        }
    }
}

#[derive(Clone)]
pub(crate) struct GtkHistoryVisitor {
    window: HostWindow,
    pub(crate) dialog: gtk::Dialog,
    close: Option<gtk::Button>,
    level: gtk::Label,
    filter: gtk::ComboBoxText,
    clear: gtk::Button,
    count: gtk::Label,
    grid: gtk::Grid,
    syncing_captions: Rc<Cell<bool>>,
    caption_filter: Rc<Cell<HistoryFilter>>,
    row: i32,
}

impl HistoryVisitor for GtkHistoryVisitor {
    fn begin(&mut self, model: &HistoryModel) {
        let captions = &model.captions;
        if let Some(close) = &self.close {
            close.set_label(&captions.close);
        }
        self.level.set_text(&captions.level);
        {
            let _sync = CaptionSyncGuard::new(&self.syncing_captions);
            self.caption_filter.set(model.filter);
            self.filter.remove_all();
            for option in &model.options {
                self.filter.append(Some(option.filter.id()), &option.label);
            }
            self.filter.set_active_id(Some(model.filter.id()));
        }
        self.clear.set_label(&captions.clear);
        self.count.set_text(&captions.count);
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        for (column, header) in captions.headers.iter().enumerate() {
            let label = gtk::Label::new(Some(header));
            label.set_xalign(0.0);
            label.add_css_class("heading");
            self.grid.attach(&label, column as i32, 0, 1, 1);
        }
        self.grid.attach(
            &gtk::Separator::new(gtk::Orientation::Horizontal),
            0,
            1,
            4,
            1,
        );
        self.row = 2;
        if model.rows.is_empty() {
            let empty = gtk::Label::new(Some(&captions.empty));
            empty.set_xalign(0.0);
            self.grid.attach(&empty, 0, self.row, 4, 1);
        }
    }

    fn row(&mut self, row: &HistoryRowView) {
        let (r, g, b) = row.rgb;
        let datetime = gtk::Label::new(None);
        datetime.set_markup(&format!(
            "<span font_family=\"monospace\" size=\"smaller\">{}</span>",
            glib::markup_escape_text(&row.datetime),
        ));
        let level = gtk::Label::new(None);
        level.set_markup(&format!(
            "<span foreground=\"#{r:02X}{g:02X}{b:02X}\">{}</span>",
            glib::markup_escape_text(row.level),
        ));
        let module = gtk::Label::new(None);
        module.set_markup(&format!(
            "<span font_family=\"monospace\" size=\"smaller\">{}</span>",
            glib::markup_escape_text(&row.module),
        ));
        module.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let message = gtk::Label::new(Some(&row.message));
        message.set_wrap(true);
        message.set_hexpand(true);
        for (column, label) in [&datetime, &level, &module, &message]
            .into_iter()
            .enumerate()
        {
            label.set_xalign(0.0);
            self.grid.attach(label, column as i32, self.row, 1, 1);
        }
        self.row += 1;
    }

    fn end(&mut self) {}
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
    #[cfg(test)]
    controller: Rc<RefCell<HistoryController>>,
    #[cfg(test)]
    delivery: Rc<HistoryDelivery>,
}

impl HistoryDialog {
    pub(crate) fn build(
        model: &HostWindow,
        window: &gtk::ApplicationWindow,
        state: &Rc<RefCell<State>>,
    ) -> HistoryDialog {
        let dialog = gtk::Dialog::with_buttons(
            Some(&model.title),
            Some(window),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[("", gtk::ResponseType::Close)],
        );
        // Toolbar: filter plus clear live together at the top. Every caption
        // goes through the i18n table (same keys as the tao twin).
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let level_caption = gtk::Label::new(None);
        level_caption.set_xalign(0.0);
        let filter = gtk::ComboBoxText::new();
        toolbar.append(&level_caption);
        toolbar.append(&filter);
        // "Clear History" names the target explicitly (cf. Event
        // Viewer "Clear Filter" vs "Clear Log"); count trails right.
        let clear = gtk::Button::new();
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
        let controller = Rc::new(RefCell::new(HistoryController::new()));
        let delivery = Rc::new(HistoryDelivery::default());
        let syncing_captions = Rc::new(Cell::new(false));
        let selected = Rc::new(Cell::new(HistoryFilter::from_id(None)));
        let caption_filter = Rc::new(Cell::new(selected.get()));
        let visitor = GtkHistoryVisitor {
            window: model.clone(),
            dialog: dialog.clone(),
            close: close_button,
            level: level_caption,
            filter: filter.clone(),
            clear: clear.clone(),
            count,
            grid: grid.clone(),
            syncing_captions: syncing_captions.clone(),
            caption_filter: caption_filter.clone(),
            row: 2,
        };
        let refresh = Rc::new({
            let state = Rc::clone(state);
            let controller = controller.clone();
            let delivery = delivery.clone();
            let selected = selected.clone();
            move || {
                // Skip refresh on a destroyed native surface; never-realized dialogs still refresh.
                if crate::surface_state::surface_dead(&visitor.dialog) {
                    return;
                }
                let model = {
                    let mut state = state.borrow_mut();
                    controller
                        .borrow_mut()
                        .update(state.host.session_mut(), selected.get())
                };
                // Release State and controller before any native setter.
                if let Some(model) = model {
                    delivery.submit(model, &mut visitor.clone());
                }
            }
        });
        refresh();
        filter.connect_changed({
            let refresh = Rc::clone(&refresh);
            let syncing_captions = Rc::clone(&syncing_captions);
            move |filter| {
                let active = filter.active_id();
                let next = HistoryFilter::from_id(active.as_deref());
                // Ignore only our temporary None / programmed selection. A
                // different selection from a synchronous callback is real input.
                if syncing_captions.get() && (active.is_none() || next == caption_filter.get()) {
                    return;
                }
                selected.set(next);
                refresh()
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
                if crate::surface_state::surface_gone(&follow_dialog) {
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
            #[cfg(test)]
            controller,
            #[cfg(test)]
            delivery,
        }
    }
}
#[cfg(test)]
pub(crate) mod history_dialog_tests {
    use super::{HistoryController, HistoryFilter, HistoryModel};
    use gtk::prelude::*;
    use nerust_settings_core::model::language::AppLanguage;

    pub(crate) fn build_history(
        parent: &gtk::ApplicationWindow,
        state: &std::rc::Rc<std::cell::RefCell<crate::State>>,
    ) -> super::HistoryDialog {
        let model = nerust_frontend_host::window::Window::auxiliary(
            nerust_frontend_host::focus::AuxKind::History,
        );
        let Some(crate::window::RenderedDialog::History(built)) =
            crate::window::render_dialog(&model, parent, state.clone(), None, || {})
        else {
            panic!("history contents")
        };
        built
    }

    /// Caption table follows the language; display application rides `refresh`.
    #[test]
    fn history_captions_follow_language() {
        let en = HistoryModel::build(&[], AppLanguage::English, HistoryFilter::from_id(None));
        let ja = HistoryModel::build(&[], AppLanguage::Japanese, HistoryFilter::from_id(None));
        for model in [&en, &ja] {
            let captions = &model.captions;
            for text in [
                &captions.title,
                &captions.close,
                &captions.level,
                &captions.clear,
                &captions.count,
                &captions.empty,
            ]
            .into_iter()
            .chain(captions.headers.iter())
            {
                assert!(!text.is_empty());
            }
            assert!(model.options.iter().all(|option| !option.label.is_empty()));
        }
        assert_ne!(en.captions.title, ja.captions.title);
        assert_ne!(en.captions.clear, ja.captions.clear);
        assert_ne!(en.options[0].label, ja.options[0].label);
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
        let built = build_history(&window, &state);
        (built.refresh)();
        // Filter to warnings on an empty history: still the empty state.
        built.filter.set_active_id(Some("warning"));
        (built.refresh)();
        built.clear.emit_clicked();
        (built.refresh)();
        built.dialog.destroy();
        // Reopen pattern: a fresh build works after destroy.
        let rebuilt = build_history(&window, &state);
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
        let built = build_history(&window, &state);
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
    fn host_history_controller_gates_language_revision_and_filter() {
        let state = model_state();
        let mut controller = HistoryController::new();
        let all = HistoryFilter::from_id(None);
        let update = |controller: &mut HistoryController, filter| {
            controller.update(state.borrow_mut().host.session_mut(), filter)
        };
        assert!(update(&mut controller, all).unwrap().rows.is_empty());
        assert!(update(&mut controller, all).is_none());
        state.borrow_mut().host.report_host_error("new row".into());
        assert_eq!(update(&mut controller, all).unwrap().rows.len(), 1);
        assert!(update(&mut controller, all).is_none());
        let mut settings = state.borrow().settings_snapshot().clone();
        settings.shared.general.language = AppLanguage::Japanese;
        state.borrow_mut().host.apply_settings(settings).unwrap();
        assert_eq!(
            update(&mut controller, all).unwrap().language,
            AppLanguage::Japanese
        );
        let error = HistoryFilter::from_id(Some("error"));
        assert!(update(&mut controller, error).is_some());
        assert!(update(&mut controller, error).is_none());
        controller.invalidate();
        assert!(update(&mut controller, error).is_some());
        state.borrow_mut().clear_notification_history();
        assert!(update(&mut controller, error).unwrap().rows.is_empty());
    }

    fn assert_host_model(built: &super::HistoryDialog, model: &HistoryModel) {
        assert_eq!(
            built.dialog.title().as_deref(),
            Some(model.captions.title.as_str())
        );
        assert_eq!(
            built.clear.label().as_deref(),
            Some(model.captions.clear.as_str())
        );
        let close = built
            .dialog
            .widget_for_response(gtk::ResponseType::Close)
            .and_downcast::<gtk::Button>()
            .unwrap();
        assert_eq!(
            close.label().as_deref(),
            Some(model.captions.close.as_str())
        );
        assert_eq!(built.filter.active_id().as_deref(), Some(model.filter.id()));
        let selected = model
            .options
            .iter()
            .find(|option| option.filter == model.filter)
            .unwrap();
        assert_eq!(
            built.filter.active_text().as_deref(),
            Some(selected.label.as_str())
        );
        let toolbar = built.clear.parent().unwrap();
        let level = toolbar.first_child().and_downcast::<gtk::Label>().unwrap();
        let count = toolbar.last_child().and_downcast::<gtk::Label>().unwrap();
        assert_eq!(level.text().as_str(), model.captions.level);
        assert_eq!(count.text().as_str(), model.captions.count);
        for (column, header) in model.captions.headers.iter().enumerate() {
            let label = built
                .grid
                .child_at(column as i32, 0)
                .and_downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(label.text().as_str(), header);
        }
        if model.rows.is_empty() {
            let empty = built
                .grid
                .child_at(0, 2)
                .and_downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(empty.text().as_str(), model.captions.empty);
        }
        for (index, row) in model.rows.iter().enumerate() {
            for (column, text) in [
                row.datetime.as_str(),
                row.level,
                row.module.as_str(),
                row.message.as_str(),
            ]
            .into_iter()
            .enumerate()
            {
                let label = built
                    .grid
                    .child_at(column as i32, index as i32 + 2)
                    .and_downcast::<gtk::Label>()
                    .unwrap();
                assert_eq!(label.text().as_str(), text);
            }
        }
    }

    /// Exercise actual GTK setters, not a fake visitor. The newer model is
    /// submitted synchronously while the outer title/caption traversal runs.
    #[test]
    fn native_history_title_reentry_is_borrow_free_and_latest_model_wins() {
        use std::{cell::Cell, rc::Rc};
        let Some((_app, window, state)) = harness() else {
            eprintln!("SKIP: history widget test needs a display (run isolated under Xvfb)");
            return;
        };
        let mut settings = state.borrow().settings_snapshot().clone();
        settings.shared.general.language = AppLanguage::English;
        state
            .borrow_mut()
            .host
            .apply_settings(settings.clone())
            .unwrap();
        let built = build_history(&window, &state);
        let empty = HistoryController::new()
            .update(
                state.borrow_mut().host.session_mut(),
                HistoryFilter::from_id(None),
            )
            .unwrap();
        assert_host_model(&built, &empty);
        let called = Rc::new(Cell::new(false));
        let signal = built.dialog.connect_title_notify({
            let called = called.clone();
            let state = state.clone();
            let controller = built.controller.clone();
            let delivery = built.delivery.clone();
            let refresh = built.refresh.clone();
            let filter = built.filter.clone();
            move |_| {
                if called.replace(true) {
                    return;
                }
                assert!(state.try_borrow_mut().is_ok());
                assert!(controller.try_borrow_mut().is_ok());
                assert!(delivery.pending.try_borrow_mut().is_ok());
                let mut settings = state.borrow().settings_snapshot().clone();
                settings.shared.general.language = AppLanguage::English;
                state.borrow_mut().host.apply_settings(settings).unwrap();
                state
                    .borrow_mut()
                    .host
                    .report_host_error("newest <row> & detail".into());
                filter.set_active_id(Some("error"));
                refresh();
            }
        });
        settings.shared.general.language = AppLanguage::Japanese;
        state.borrow_mut().host.apply_settings(settings).unwrap();
        (built.refresh)();
        assert!(called.get(), "native title notify must actually execute");
        let model = HistoryController::new()
            .update(
                state.borrow_mut().host.session_mut(),
                HistoryFilter::from_id(Some("error")),
            )
            .unwrap();
        assert!(!model.rows.is_empty());
        assert_host_model(&built, &model);
        built.dialog.disconnect(signal);
        built.dialog.destroy();
        window.destroy();
    }

    #[test]
    fn native_history_filter_reentry_during_caption_sync_keeps_newest_selection() {
        use std::{cell::Cell, rc::Rc};
        let Some((_app, window, state)) = harness() else {
            eprintln!("SKIP: history widget test needs a display (run isolated under Xvfb)");
            return;
        };
        let built = build_history(&window, &state);
        let called = Rc::new(Cell::new(false));
        let signal = built.filter.connect_changed({
            let called = called.clone();
            let state = state.clone();
            let controller = built.controller.clone();
            let delivery = built.delivery.clone();
            move |filter| {
                if filter.active_id().as_deref() != Some("all") || called.replace(true) {
                    return;
                }
                assert!(state.try_borrow_mut().is_ok());
                assert!(controller.try_borrow_mut().is_ok());
                assert!(delivery.pending.try_borrow_mut().is_ok());
                // This invokes production's changed handler while caption-sync
                // is armed, and must enqueue rather than silently drop input.
                filter.set_active_id(Some("error"));
            }
        });
        state
            .borrow_mut()
            .host
            .report_host_error("filter callback".into());
        (built.refresh)();
        assert!(
            called.get(),
            "native changed callback must actually execute"
        );
        let model = HistoryController::new()
            .update(
                state.borrow_mut().host.session_mut(),
                HistoryFilter::from_id(Some("error")),
            )
            .unwrap();
        assert_host_model(&built, &model);
        built.filter.disconnect(signal);
        built.dialog.destroy();
        window.destroy();
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
        let en_captions =
            HistoryModel::build(&[], AppLanguage::English, HistoryFilter::from_id(None)).captions;
        let ja_captions =
            HistoryModel::build(&[], AppLanguage::Japanese, HistoryFilter::from_id(None)).captions;
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
            HistoryModel::build(&lines, AppLanguage::English, HistoryFilter::from_id(active))
                .rows
                .len()
        };
        assert_eq!(visible(Some("all")), 2);
        assert_eq!(visible(Some("warning")), 1);
        assert_eq!(visible(Some("error")), 1);
    }
}

#[cfg(test)]
mod tests {
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
        let update = state.borrow_mut().host.prepare_osd(size, 240, &ctx, now);
        update.apply(&mut sink);
        let update = state.borrow_mut().host.prepare_osd(size, 240, &ctx, now);
        update.apply(&mut sink);
        assert_eq!(sink.0, 1);
        state.borrow_mut().host.invalidate_osd();
        assert!(!state.borrow().host.osd_layer_live());
        let update = state.borrow_mut().host.prepare_osd(size, 240, &ctx, now);
        update.apply(&mut sink);
        assert_eq!(sink.0, 2, "renderer replacement re-pushes the strip");
        state.borrow_mut().host.invalidate_osd();
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
