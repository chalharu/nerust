use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_settings::{input::ShortcutAction, local::ScalingMode};
use nerust_gui_shell::session::{KeyboardShortcut, SessionError, access::FrontendSession};
use nerust_persistence::model::StateSlotSummary;
use nerust_render_base::renderer::GpuFactory;

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
    close_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    resume_action: gio::SimpleAction,
    state_create_action: gio::SimpleAction,
    state_save_active_action: gio::SimpleAction,
    state_load_active_action: gio::SimpleAction,
    state_select_slot_action: gio::SimpleAction,
    state_save_slot_action: gio::SimpleAction,
    state_load_slot_action: gio::SimpleAction,
    state_delete_slot_action: gio::SimpleAction,
    select_active_slot_menu: gio::Menu,
    save_slot_menu: gio::Menu,
    load_slot_menu: gio::Menu,
    delete_slot_menu: gio::Menu,
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

pub(crate) enum KeyEventState {
    Press,
    Release,
}

fn key_event_pressed(event: KeyEventState) -> bool {
    matches!(event, KeyEventState::Press)
}

fn load_active_slot(state: &RefCell<State>) -> bool {
    let active_slot_id = state.borrow().active_slot_id();
    active_slot_id
        .is_some_and(|slot_id| FrontendSession::load_slot(&mut *state.borrow_mut(), slot_id))
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
        let close_action = gio::SimpleAction::new("close", None);
        let pause_action = gio::SimpleAction::new("pause", None);
        let resume_action = gio::SimpleAction::new("resume", None);
        let state_create_action = gio::SimpleAction::new("state-create", None);
        let state_save_active_action = gio::SimpleAction::new("state-save-active", None);
        let state_load_active_action = gio::SimpleAction::new("state-load-active", None);
        let state_select_slot_action =
            gio::SimpleAction::new("state-select-slot", Some(&u64::static_variant_type()));
        let state_save_slot_action =
            gio::SimpleAction::new("state-save-slot", Some(&u64::static_variant_type()));
        let state_load_slot_action =
            gio::SimpleAction::new("state-load-slot", Some(&u64::static_variant_type()));
        let state_delete_slot_action =
            gio::SimpleAction::new("state-delete-slot", Some(&u64::static_variant_type()));
        let settings_action = gio::SimpleAction::new("settings", None);
        let _ = Surface::bind(&window, state.clone(), factory);
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state,
            open_dialog: None,
            close_action: close_action.clone(),
            pause_action: pause_action.clone(),
            resume_action: resume_action.clone(),
            state_create_action: state_create_action.clone(),
            state_save_active_action: state_save_active_action.clone(),
            state_load_active_action: state_load_active_action.clone(),
            state_select_slot_action: state_select_slot_action.clone(),
            state_save_slot_action: state_save_slot_action.clone(),
            state_load_slot_action: state_load_slot_action.clone(),
            state_delete_slot_action: state_delete_slot_action.clone(),
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
        window.add_action(&close_action);

        {
            let result = result.clone();
            let _ = pause_action.connect_activate(move |_, _| {
                result.state().borrow_mut().pause();
                result.update_actions();
            });
        }
        window.add_action(&pause_action);

        {
            let result = result.clone();
            let _ = resume_action.connect_activate(move |_, _| {
                result.state().borrow_mut().resume();
                result.update_actions();
            });
        }
        window.add_action(&resume_action);

        {
            let result = result.clone();
            let _ = state_create_action.connect_activate(move |_, _| {
                result.state().borrow_mut().create_slot();
                result.update_actions();
            });
        }
        window.add_action(&state_create_action);

        {
            let result = result.clone();
            let _ = state_save_active_action.connect_activate(move |_, _| {
                result.state().borrow_mut().save_active_slot();
                result.update_actions();
            });
        }
        window.add_action(&state_save_active_action);

        {
            let result = result.clone();
            let _ = state_load_active_action.connect_activate(move |_, _| {
                let state = result.state();
                if load_active_slot(state.as_ref()) {
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_load_active_action);

        {
            let result = result.clone();
            let _ = state_select_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().select_slot(slot_id);
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_select_slot_action);

        {
            let result = result.clone();
            let _ = state_save_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().save_slot(slot_id);
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_save_slot_action);

        {
            let result = result.clone();
            let _ = state_load_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result.state().borrow_mut().load_slot(slot_id);
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_load_slot_action);

        {
            let result = result.clone();
            let _ = state_delete_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    result.state().borrow_mut().delete_slot(slot_id);
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_delete_slot_action);

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

    fn open(&self) {
        let file_chooser_native = gtk::FileChooserNative::new(
            Some("Open File"),
            Some(&self.window()),
            gtk::FileChooserAction::Open,
            Some("_Open"),
            Some("_Cancel"),
        );
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
        self.state().borrow_mut().load_path(path);
        self.update_actions();
    }

    fn close(&self) {
        let _ = self.state().borrow_mut().unload();
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
        self.borrow().close_action.set_enabled(state.loaded());
        self.borrow().pause_action.set_enabled(state.can_pause());
        self.borrow().resume_action.set_enabled(state.can_resume());
        self.borrow()
            .state_create_action
            .set_enabled(state.loaded());
        self.borrow()
            .state_save_active_action
            .set_enabled(state.loaded());
        self.borrow()
            .state_load_active_action
            .set_enabled(state.active_slot_id().is_some());
        self.borrow()
            .state_select_slot_action
            .set_enabled(state.loaded() && !state.slots().is_empty());
        self.borrow()
            .state_save_slot_action
            .set_enabled(state.loaded() && !state.slots().is_empty());
        self.borrow()
            .state_load_slot_action
            .set_enabled(state.loaded() && !state.slots().is_empty());
        self.borrow()
            .state_delete_slot_action
            .set_enabled(state.loaded() && !state.slots().is_empty());

        rebuild_slot_menu(
            &self.borrow().select_active_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-select-slot",
        );
        rebuild_slot_menu(
            &self.borrow().save_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-save-slot",
        );
        rebuild_slot_menu(
            &self.borrow().load_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-load-slot",
        );
        rebuild_slot_menu(
            &self.borrow().delete_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-delete-slot",
        );
        drop(state);
        self.refresh_title();
    }

    fn rebuild_menubar(&self) {
        let language = self
            .state()
            .borrow()
            .settings_snapshot()
            .shared
            .general
            .language;
        let menu_model = build_menu_model(
            language,
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
        match shortcut {
            KeyboardShortcut::Session(action) => match action {
                ShortcutAction::TogglePause => {
                    self.state().borrow_mut().toggle_pause();
                }
                ShortcutAction::SaveActiveSlot => {
                    self.state().borrow_mut().save_active_slot();
                }
                ShortcutAction::SelectNextSlot => {
                    self.state().borrow_mut().select_next_slot();
                }
                ShortcutAction::SelectPreviousSlot => {
                    self.state().borrow_mut().select_previous_slot();
                }
                ShortcutAction::LoadActiveSlot => {
                    let _ = self.state().borrow_mut().load_active_slot();
                }
                ShortcutAction::Reset => {
                    self.state().borrow_mut().reset();
                }
                ShortcutAction::ToggleFullscreen => {
                    toggle_window_fullscreen(self);
                }
            },
            KeyboardShortcut::ToggleFullscreen => {
                toggle_window_fullscreen(self);
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
) {
    menu.remove_all();
    for slot in slots {
        let item = gio::MenuItem::new(Some(&slot_label(slot, active_slot)), None);
        item.set_action_and_target_value(Some(action), Some(&slot.slot_id.to_variant()));
        menu.append_item(&item);
    }
}
