use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionError,
    access::{PauseControl, SessionControl, SlotControl},
};
use nerust_persistence::model::StateSlotSummary;
use nerust_render_traits::renderer::GpuFactory;
use nerust_settings_core::model::local::ScalingMode;

use super::{
    State, TITLE_UPDATE_INTERVAL, build_menu_model,
    surface::{Surface, SurfaceExtend},
};
use crate::preferences::present_preferences_dialog;

pub(crate) struct StateMenus {
    pub(crate) select_active_slot_menu: gio::Menu,
    pub(crate) save_slot_menu: gio::Menu,
    pub(crate) load_slot_menu: gio::Menu,
    pub(crate) delete_slot_menu: gio::Menu,
}

pub(crate) struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    open_dialog: Option<gtk::FileChooserNative>,
    debugger_window: Option<crate::debugger::DebuggerWindow>,
    history_dialog: Option<gtk::Dialog>,
    actions: WindowActions,
    select_active_slot_menu: gio::Menu,
    save_slot_menu: gio::Menu,
    load_slot_menu: gio::Menu,
    delete_slot_menu: gio::Menu,
}

/// Window action cluster: every menu/toolbar command in one place.
/// `WindowCore` owns one; wiring and gating go through these narrow ports.
pub(crate) struct WindowActions {
    pub close: gio::SimpleAction,
    pub pause: gio::SimpleAction,
    pub resume: gio::SimpleAction,
    pub reset: gio::SimpleAction,
    pub debugger: gio::SimpleAction,
    pub history: gio::SimpleAction,
    pub state_create: gio::SimpleAction,
    pub state_save_active: gio::SimpleAction,
    pub state_load_active: gio::SimpleAction,
    pub state_select_slot: gio::SimpleAction,
    pub state_save_slot: gio::SimpleAction,
    pub state_load_slot: gio::SimpleAction,
    pub state_delete_slot: gio::SimpleAction,
}

impl WindowActions {
    /// Fresh disabled-by-default actions; `update` gates them per tick.
    pub fn new() -> Self {
        Self {
            close: gio::SimpleAction::new("close", None),
            pause: gio::SimpleAction::new("pause", None),
            resume: gio::SimpleAction::new("resume", None),
            reset: gio::SimpleAction::new("reset", None),
            debugger: gio::SimpleAction::new("debugger", None),
            history: gio::SimpleAction::new("history", None),
            state_create: gio::SimpleAction::new("state-create", None),
            state_save_active: gio::SimpleAction::new("state-save-active", None),
            state_load_active: gio::SimpleAction::new("state-load-active", None),
            state_select_slot: gio::SimpleAction::new(
                "state-select-slot",
                Some(&u64::static_variant_type()),
            ),
            state_save_slot: gio::SimpleAction::new(
                "state-save-slot",
                Some(&u64::static_variant_type()),
            ),
            state_load_slot: gio::SimpleAction::new(
                "state-load-slot",
                Some(&u64::static_variant_type()),
            ),
            state_delete_slot: gio::SimpleAction::new(
                "state-delete-slot",
                Some(&u64::static_variant_type()),
            ),
        }
    }

    /// Seat every action on the window: one call instead of twelve.
    pub fn add_to(&self, window: &gtk::ApplicationWindow) {
        window.add_action(&self.close);
        window.add_action(&self.pause);
        window.add_action(&self.resume);
        window.add_action(&self.reset);
        window.add_action(&self.debugger);
        window.add_action(&self.history);
        window.add_action(&self.state_create);
        window.add_action(&self.state_save_active);
        window.add_action(&self.state_load_active);
        window.add_action(&self.state_select_slot);
        window.add_action(&self.state_save_slot);
        window.add_action(&self.state_load_slot);
        window.add_action(&self.state_delete_slot);
    }
}

pub(crate) type Window = Rc<RefCell<WindowCore>>;

pub(crate) trait WindowExtend {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn GpuFactory>,
        state_menus: StateMenus,
    ) -> Window;
    fn window(&self) -> gtk::ApplicationWindow;
    fn application(&self) -> gtk::Application;
    fn state(&self) -> Rc<RefCell<State>>;
    fn open_debugger(&self);
    fn open_history(&self);
    fn realize(&self);
    fn close_request(&self) -> bool;
    fn open(&self);
    fn load_path(&self, path: &Path);
    fn close(&self);
    fn update_actions(&self);
    fn rebuild_menubar(&self);
    fn refresh_title(&self);
    fn sync_fullscreen_from_settings(&self);
    fn key_event(&self, key: gdk::Key, enevt: KeyEventState) -> bool;
    fn apply_keyboard_shortcut(&self, shortcut: KeyboardShortcut);
}

#[derive(Clone, Copy)]
pub(crate) enum KeyEventState {
    Press,
    Release,
}

fn key_event_pressed(event: KeyEventState) -> bool {
    matches!(event, KeyEventState::Press)
}

fn load_active_slot(state: &RefCell<State>) -> bool {
    let active_slot_id = state.borrow().active_slot_id();
    active_slot_id.is_some_and(|slot_id| SlotControl::load_slot(&mut *state.borrow_mut(), slot_id))
}

impl WindowExtend for Window {
    fn state(&self) -> Rc<RefCell<State>> {
        self.borrow().state.clone()
    }

    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn GpuFactory>,
        state_menus: StateMenus,
    ) -> Window {
        let actions = WindowActions::new();
        let close_action = actions.close.clone();
        let pause_action = actions.pause.clone();
        let resume_action = actions.resume.clone();
        let reset_action = actions.reset.clone();
        let debugger_action = actions.debugger.clone();
        let history_action = actions.history.clone();
        let state_create_action = actions.state_create.clone();
        let state_save_active_action = actions.state_save_active.clone();
        let state_load_active_action = actions.state_load_active.clone();
        let state_select_slot_action = actions.state_select_slot.clone();
        let state_save_slot_action = actions.state_save_slot.clone();
        let state_load_slot_action = actions.state_load_slot.clone();
        let state_delete_slot_action = actions.state_delete_slot.clone();
        let settings_action = gio::SimpleAction::new("settings", None);
        let _ = Surface::bind(&window, state.clone(), factory);
        actions.add_to(&window);
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state,
            open_dialog: None,
            debugger_window: None,
            history_dialog: None,
            actions,
            select_active_slot_menu: state_menus.select_active_slot_menu,
            save_slot_menu: state_menus.save_slot_menu,
            load_slot_menu: state_menus.load_slot_menu,
            delete_slot_menu: state_menus.delete_slot_menu,
        }));
        {
            let result = result.clone();
            let _ = window.connect_realize(move |_window| result.realize());
        }
        {
            let result = result.clone();
            let _ = window
                .connect_close_request(move |_| glib::Propagation::from(result.close_request()));
        }
        {
            let result = result.clone();
            let _ = window.connect_is_active_notify(move |window| {
                if !window.is_active() {
                    result.state().borrow_mut().clear_input();
                }
            });
        }
        {
            let result = result.clone();
            let _ = window.connect_notify_local(Some("fullscreened"), move |window, _| {
                sync_persisted_window_fullscreen(&result, window.is_fullscreen());
            });
        }

        let key_controller = gtk::EventControllerKey::new();
        {
            let result = result.clone();
            let _ = key_controller.connect_key_pressed(move |_, key, _, _| {
                glib::Propagation::from(result.key_event(key, KeyEventState::Press))
            });
        }
        {
            let result = result.clone();
            let _ = key_controller.connect_key_released(move |_, key, _, _| {
                result.key_event(key, KeyEventState::Release);
            });
        }
        window.add_controller(key_controller);

        let open_action = gio::SimpleAction::new("open", None);

        {
            let result = result.clone();
            let _ = open_action.connect_activate(move |_, _| {
                result.open();
            });
        }
        window.add_action(&open_action);

        {
            let result = result.clone();
            let _ = close_action.connect_activate(move |_, _| {
                result.close();
            });
        }

        {
            let result = result.clone();
            let _ = pause_action.connect_activate(move |_, _| {
                result.state().borrow_mut().pause();
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = resume_action.connect_activate(move |_, _| {
                result.state().borrow_mut().resume();
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = reset_action.connect_activate(move |_, _| {
                result.state().borrow_mut().reset();
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = debugger_action.connect_activate(move |_, _| {
                result.open_debugger();
            });
        }

        {
            let result = result.clone();
            let _ = history_action.connect_activate(move |_, _| {
                result.open_history();
            });
        }

        {
            let result = result.clone();
            let _ = state_create_action.connect_activate(move |_, _| {
                result.state().borrow_mut().create_slot();
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = state_save_active_action.connect_activate(move |_, _| {
                result.state().borrow_mut().save_active_slot();
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = state_load_active_action.connect_activate(move |_, _| {
                let state = result.state();
                if load_active_slot(state.as_ref()) {
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = state_select_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().select_slot(slot_id);
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = state_save_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().save_slot(slot_id);
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = state_load_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result.state().borrow_mut().load_slot(slot_id);
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = state_delete_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().delete_slot(slot_id);
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = settings_action.connect_activate(move |_, _| {
                let was_running = !result.state().borrow().paused();
                result.state().borrow_mut().pause();
                let state = result.state();
                let window = result.window();
                let result_for_close = result.clone();
                present_preferences_dialog(&window, state, move || {
                    if was_running {
                        result_for_close.state().borrow_mut().resume();
                    }
                    result_for_close.rebuild_menubar();
                    result_for_close.update_actions();
                });
            });
        }
        window.add_action(&settings_action);

        {
            let result = result.clone();
            let _ = glib::timeout_add_local(TITLE_UPDATE_INTERVAL, move || {
                result.refresh_title();
                glib::ControlFlow::Continue
            });
        }

        result.rebuild_menubar();

        result.update_actions();
        window.present();
        result.sync_fullscreen_from_settings();
        let _ = window.grab_focus();
        result
    }

    fn open_history(&self) {
        // Parity with the debugger: an existing dialog presents
        // instead of duplicating; destroy clears the slot.
        if let Some(existing) = self.borrow().history_dialog.clone() {
            existing.present();
            return;
        }
        let dialog = crate::osd::GtkOsd::build_history(&self.window(), &self.state());
        {
            let result = self.clone();
            dialog.connect_destroy(move |_| {
                result.borrow_mut().history_dialog = None;
            });
        }
        self.borrow_mut().history_dialog = Some(dialog.clone());
        dialog.present();
    }

    fn open_debugger(&self) {
        // Idempotent open: an existing window refreshes and presents
        // instead of duplicating.
        if let Some(existing) = self.borrow().debugger_window.clone() {
            crate::debugger::DebuggerWindowCore::refresh(&existing);
            existing.borrow().present();
            return;
        }
        let debugger = crate::debugger::DebuggerWindowCore::open(
            &self.application(),
            &self.window(),
            self.state(),
        );
        self.borrow_mut().debugger_window = Some(debugger);
        self.update_actions();
    }

    fn open(&self) {
        let file_chooser_native = gtk::FileChooserNative::new(
            Some("Open File"),
            Some(&self.window()),
            gtk::FileChooserAction::Open,
            Some("_Open"),
            Some("_Cancel"),
        );
        for factory in self.state().borrow().session.registry().all() {
            let extensions = factory.supported_extensions();
            if extensions.is_empty() {
                continue;
            }
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&format!("{} ROM", factory.display_name())));
            for extension in extensions {
                filter.add_pattern(&format!("*.{extension}"));
            }
            file_chooser_native.add_filter(&filter);
        }
        let result = self.clone();
        let _ = file_chooser_native.connect_response(move |file_chooser_native, response| {
            if response == gtk::ResponseType::Accept
                && let Some(path) = file_chooser_native.file().and_then(|f| f.path())
            {
                result.load_path(&path);
            }

            file_chooser_native.hide();
            result.borrow_mut().open_dialog = None;
            result.update_actions();

            let window = result.window();
            if let Some(root) = gtk::prelude::GtkWindowExt::focus(&window) {
                let _ = root.grab_focus();
            } else {
                let _ = window.grab_focus();
            }
        });
        self.borrow_mut().open_dialog = Some(file_chooser_native.clone());
        file_chooser_native.show();
    }

    fn load_path(&self, path: &Path) {
        // End the state borrow before presenting: the dialog reads settings back
        // through the same RefCell, so holding the guard would double-borrow.
        let detail = self.state().borrow_mut().load_path(path).err();
        if let Some(detail) = detail {
            show_rom_load_error(&self.window(), self.state(), &detail);
        }
        self.update_actions();
    }

    fn close(&self) {
        self.state().borrow_mut().unload();
        self.update_actions();
    }

    fn realize(&self) {}

    fn close_request(&self) -> bool {
        self.state().borrow_mut().flush_before_exit();
        let w = self.window().width();
        let h = self.window().height();
        if w > 0 && h > 0 {
            let state = self.state();
            let _ = state
                .borrow()
                .session
                .settings_manager()
                .update_window_size(w as u32, h as u32);
        }
        self.application().quit();
        false
    }

    fn window(&self) -> gtk::ApplicationWindow {
        self.borrow().window.clone()
    }

    fn application(&self) -> gtk::Application {
        self.borrow().application.clone()
    }

    fn update_actions(&self) {
        let state = self.state();
        let state = state.borrow();
        let tick = state.session.tick_metrics();
        self.borrow().actions.close.set_enabled(tick.loaded);
        self.borrow().actions.pause.set_enabled(tick.can_pause());
        self.borrow().actions.resume.set_enabled(tick.can_resume());
        self.borrow().actions.reset.set_enabled(tick.loaded);
        // Coreless open is a designed state: gating on `loaded` would disagree
        // with the shortcut, so the action stays enabled.
        self.borrow().actions.debugger.set_enabled(true);
        self.borrow().actions.history.set_enabled(true);
        self.borrow().actions.state_create.set_enabled(tick.loaded);
        self.borrow()
            .actions
            .state_save_active
            .set_enabled(tick.loaded);
        self.borrow()
            .actions
            .state_load_active
            .set_enabled(state.active_slot_id().is_some());
        self.borrow()
            .actions
            .state_select_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_save_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_load_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_delete_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());

        let language = state.settings_snapshot().shared.general.language;
        rebuild_slot_menu(
            &self.borrow().select_active_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-select-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().save_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-save-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().load_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-load-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().delete_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-delete-slot",
            language,
        );
        drop(state);
        self.refresh_title();
    }

    fn rebuild_menubar(&self) {
        let state = self.state();
        let guarded = state.borrow();
        let snapshot = guarded.settings_snapshot();
        let menu_model = build_menu_model(
            snapshot.shared.general.language,
            &snapshot.shared.input.shortcuts.keyboard,
            &gio::Menu::new(),
            &self.borrow().select_active_slot_menu,
            &self.borrow().save_slot_menu,
            &self.borrow().load_slot_menu,
            &self.borrow().delete_slot_menu,
        );
        self.application().set_menubar(Some(&menu_model));
    }

    fn refresh_title(&self) {
        let title = self.state().borrow().title();
        self.window().set_title(Some(title.as_str()));
    }

    fn sync_fullscreen_from_settings(&self) {
        let state = self.state();
        let snapshot = state.borrow().settings_snapshot().clone();
        let fullscreen = snapshot.local.video.window.fullscreen_default;

        // Restore remembered window size when not in fullscreen or scaling mode.
        if !fullscreen
            && snapshot.local.video.window.scaling == ScalingMode::FitToWindow
            && let Some(size) = snapshot.app_state.window_size("main")
        {
            self.window()
                .set_default_size(size.width as i32, size.height as i32);
        }

        set_window_fullscreen(&self.window(), fullscreen);
    }

    fn key_event(&self, key: gdk::Key, event: KeyEventState) -> bool {
        // No hardcoded F-keys: the debugger open rides the shortcut schema
        // (default F4, rebindable in Settings), so a user binding always wins.
        if let Ok(controller_input) = key.try_into() {
            let shortcut = self
                .state()
                .borrow_mut()
                .handle_keyboard_key(controller_input, key_event_pressed(event));
            if let Some(shortcut) = shortcut {
                self.apply_keyboard_shortcut(shortcut);
            }
        }
        false
    }

    fn apply_keyboard_shortcut(&self, shortcut: KeyboardShortcut) {
        use nerust_gui_shell::session::access::dispatch_session_action;
        match shortcut {
            KeyboardShortcut::Session(action) => {
                let _ = dispatch_session_action(&mut *self.state().borrow_mut(), action);
            }
            KeyboardShortcut::ToggleFullscreen => {
                toggle_window_fullscreen(self);
            }
            KeyboardShortcut::OpenDebugger => {
                self.open_debugger();
            }
        }
        self.update_actions();
    }
}

fn set_window_fullscreen(window: &gtk::ApplicationWindow, fullscreen: bool) {
    window.set_fullscreened(fullscreen);
}

fn persist_window_fullscreen_default(
    window: &Window,
    fullscreen: bool,
) -> Result<bool, SessionError> {
    let state = window.state();
    Ok(state
        .borrow_mut()
        .set_fullscreen_default(fullscreen)?
        .fullscreen_default_changed)
}

fn sync_persisted_window_fullscreen(window: &Window, fullscreen: bool) {
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(true) => window.update_actions(),
        Ok(false) => (),
        Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
    }
}

fn apply_persisted_window_fullscreen(window: &Window, fullscreen: bool) {
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(_) => window.sync_fullscreen_from_settings(),
        Err(error) => {
            log::warn!("failed to persist fullscreen setting: {error}");
            set_window_fullscreen(&window.window(), fullscreen);
        }
    }
}

fn toggle_window_fullscreen(window: &Window) {
    let fullscreen = !window.window().is_fullscreen();
    apply_persisted_window_fullscreen(window, fullscreen);
}

fn rebuild_slot_menu(
    menu: &gio::Menu,
    slots: &[StateSlotSummary],
    active_slot: Option<u64>,
    action: &str,
    language: nerust_settings_core::model::language::AppLanguage,
) {
    use nerust_settings_core::i18n::{UiText, text};
    menu.remove_all();
    if slots.is_empty() {
        // An action-less item renders disabled: the empty submenu
        // explains itself instead of showing a blank list.
        let item = gio::MenuItem::new(Some(text(language, UiText::NoSaveSlots)), None);
        menu.append_item(&item);
        return;
    }
    for slot in slots {
        let item = gio::MenuItem::new(Some(&slot_label(slot, active_slot)), None);
        item.set_action_and_target_value(Some(action), Some(&slot.slot_id.to_variant()));
        menu.append_item(&item);
    }
}

/// Modal error dialog for ROM load failures: mirrors the tao frontend so a
/// bad file explains itself instead of failing silently.
fn show_rom_load_error(parent: &gtk::ApplicationWindow, state: Rc<RefCell<State>>, detail: &str) {
    use gtk::prelude::*;
    use nerust_settings_core::i18n::{UiText, text};
    let language = state.borrow().settings_snapshot().shared.general.language;
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .message_type(gtk::MessageType::Error)
        .buttons(gtk::ButtonsType::Close)
        .text(text(language, UiText::RomLoadFailed))
        .secondary_text(detail)
        .build();
    let dialog_clone = dialog.clone();
    let _ = dialog.connect_response(move |_, _| {
        dialog_clone.hide();
    });
    dialog.present();
}

#[cfg(test)]
mod tests {
    use gio::prelude::ActionExt as _;

    use super::{KeyEventState, WindowActions, key_event_pressed, rebuild_slot_menu};

    #[test]
    fn key_press_state_maps_to_pressed_flag() {
        assert!(key_event_pressed(KeyEventState::Press));
        assert!(!key_event_pressed(KeyEventState::Release));
    }

    #[test]
    fn action_cluster_names_every_command() {
        // One home for all command names: renaming a command updates
        // construction and wiring together, never just one side.
        let actions = WindowActions::new();
        for (action, name) in [
            (&actions.close, "close"),
            (&actions.pause, "pause"),
            (&actions.resume, "resume"),
            (&actions.reset, "reset"),
            (&actions.debugger, "debugger"),
            (&actions.history, "history"),
            (&actions.state_create, "state-create"),
            (&actions.state_save_active, "state-save-active"),
            (&actions.state_load_active, "state-load-active"),
            (&actions.state_select_slot, "state-select-slot"),
            (&actions.state_save_slot, "state-save-slot"),
            (&actions.state_load_slot, "state-load-slot"),
            (&actions.state_delete_slot, "state-delete-slot"),
        ] {
            assert_eq!(action.name().as_str(), name);
        }
    }

    fn slot_menu_count(menu: &gio::Menu) -> i32 {
        use gio::prelude::MenuModelExt;
        menu.n_items()
    }

    #[test]
    fn slot_menu_explains_empty_state() {
        use nerust_settings_core::model::language::AppLanguage;
        let menu = gio::Menu::new();
        rebuild_slot_menu(
            &menu,
            &[],
            None,
            "win.state-save-slot",
            AppLanguage::English,
        );
        // One disabled item names the empty state, never a blank list.
        assert_eq!(slot_menu_count(&menu), 1);
    }

    #[test]
    fn slot_menu_lists_slots_with_action_targets() {
        use nerust_persistence::model::StateSlotSummary;
        use nerust_settings_core::model::language::AppLanguage;
        let slots = vec![
            StateSlotSummary {
                schema_version: 1,
                slot_id: 1,
                path: std::path::PathBuf::from("slot1"),
                saved_at: std::time::SystemTime::UNIX_EPOCH,
                has_thumbnail: false,
                emulator_version: "test".to_string(),
            },
            StateSlotSummary {
                schema_version: 1,
                slot_id: 2,
                path: std::path::PathBuf::from("slot2"),
                saved_at: std::time::SystemTime::UNIX_EPOCH,
                has_thumbnail: false,
                emulator_version: "test".to_string(),
            },
        ];
        let menu = gio::Menu::new();
        rebuild_slot_menu(
            &menu,
            &slots,
            Some(2),
            "win.state-load-slot",
            AppLanguage::English,
        );
        assert_eq!(slot_menu_count(&menu), 2);
    }
}
