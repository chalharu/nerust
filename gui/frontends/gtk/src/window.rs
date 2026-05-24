use super::glarea::{GLArea, GLAreaExtend};
use super::preferences::present_preferences_dialog;
use super::{State, TITLE_UPDATE_INTERVAL};
use gtk::gio;
use gtk::glib;
use gtk::glib::variant::{StaticVariantType, ToVariant};
use gtk::prelude::*;
use nerust_contract_settings::{input::KeyboardKey, shortcut::ShortcutAction};
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_shell::settings::{
    bindings::events::{
        controller::controller_event_for_key,
        shortcut::{shortcut_action_for_key, shortcut_command_for_key},
    },
    defaults::manager::current_or_default,
};
use nerust_persistence::model::StateSlotSummary;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::rc::Rc;

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
    fullscreened: bool,
}

pub(crate) type Window = Rc<RefCell<WindowCore>>;

pub(crate) trait WindowExtend {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        glarea: gtk::GLArea,
        state: Rc<RefCell<State>>,
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
    fn refresh_title(&self);
    fn key_event(&self, key: gdk::Key, enevt: KeyEventState) -> bool;
}

pub(crate) enum KeyEventState {
    Press,
    Release,
}

fn gdk_key_settings_key(key: gdk::Key) -> Option<KeyboardKey> {
    Some(match key {
        gdk::Key::a | gdk::Key::A => KeyboardKey::KeyA,
        gdk::Key::b | gdk::Key::B => KeyboardKey::KeyB,
        gdk::Key::c | gdk::Key::C => KeyboardKey::KeyC,
        gdk::Key::d | gdk::Key::D => KeyboardKey::KeyD,
        gdk::Key::e | gdk::Key::E => KeyboardKey::KeyE,
        gdk::Key::f | gdk::Key::F => KeyboardKey::KeyF,
        gdk::Key::g | gdk::Key::G => KeyboardKey::KeyG,
        gdk::Key::h | gdk::Key::H => KeyboardKey::KeyH,
        gdk::Key::i | gdk::Key::I => KeyboardKey::KeyI,
        gdk::Key::j | gdk::Key::J => KeyboardKey::KeyJ,
        gdk::Key::k | gdk::Key::K => KeyboardKey::KeyK,
        gdk::Key::l | gdk::Key::L => KeyboardKey::KeyL,
        gdk::Key::m | gdk::Key::M => KeyboardKey::KeyM,
        gdk::Key::n | gdk::Key::N => KeyboardKey::KeyN,
        gdk::Key::o | gdk::Key::O => KeyboardKey::KeyO,
        gdk::Key::p | gdk::Key::P => KeyboardKey::KeyP,
        gdk::Key::q | gdk::Key::Q => KeyboardKey::KeyQ,
        gdk::Key::r | gdk::Key::R => KeyboardKey::KeyR,
        gdk::Key::s | gdk::Key::S => KeyboardKey::KeyS,
        gdk::Key::t | gdk::Key::T => KeyboardKey::KeyT,
        gdk::Key::u | gdk::Key::U => KeyboardKey::KeyU,
        gdk::Key::v | gdk::Key::V => KeyboardKey::KeyV,
        gdk::Key::w | gdk::Key::W => KeyboardKey::KeyW,
        gdk::Key::x | gdk::Key::X => KeyboardKey::KeyX,
        gdk::Key::y | gdk::Key::Y => KeyboardKey::KeyY,
        gdk::Key::z | gdk::Key::Z => KeyboardKey::KeyZ,
        gdk::Key::Up => KeyboardKey::ArrowUp,
        gdk::Key::Down => KeyboardKey::ArrowDown,
        gdk::Key::Left => KeyboardKey::ArrowLeft,
        gdk::Key::Right => KeyboardKey::ArrowRight,
        gdk::Key::Return => KeyboardKey::Enter,
        gdk::Key::Escape => KeyboardKey::Escape,
        gdk::Key::space => KeyboardKey::Space,
        gdk::Key::Tab => KeyboardKey::Tab,
        gdk::Key::F1 => KeyboardKey::F1,
        gdk::Key::F2 => KeyboardKey::F2,
        gdk::Key::F3 => KeyboardKey::F3,
        gdk::Key::F4 => KeyboardKey::F4,
        gdk::Key::F5 => KeyboardKey::F5,
        gdk::Key::F6 => KeyboardKey::F6,
        gdk::Key::F7 => KeyboardKey::F7,
        gdk::Key::F8 => KeyboardKey::F8,
        gdk::Key::F9 => KeyboardKey::F9,
        gdk::Key::F10 => KeyboardKey::F10,
        gdk::Key::F11 => KeyboardKey::F11,
        gdk::Key::F12 => KeyboardKey::F12,
        _ => return None,
    })
}

fn key_event_pressed(event: KeyEventState) -> bool {
    matches!(event, KeyEventState::Press)
}

trait ActiveSlotLoader {
    fn active_slot_id(&self) -> Option<u64>;
    fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome;
}

impl ActiveSlotLoader for State {
    fn active_slot_id(&self) -> Option<u64> {
        State::active_slot_id(self)
    }

    fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        State::run_command(self, command)
    }
}

fn load_active_slot<T: ActiveSlotLoader>(state: &RefCell<T>) -> bool {
    let active_slot_id = { state.borrow().active_slot_id() };
    if let Some(slot_id) = active_slot_id {
        state
            .borrow_mut()
            .run_command(SessionCommand::LoadSlot(slot_id))
            .executed
    } else {
        false
    }
}

impl WindowExtend for Window {
    fn state(&self) -> Rc<RefCell<State>> {
        self.borrow().state.clone()
    }

    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        glarea: gtk::GLArea,
        state: Rc<RefCell<State>>,
        state_menus: StateMenus,
    ) -> Window {
        let close_action = gio::SimpleAction::new("close", None);
        let pause_action = gio::SimpleAction::new("pause", None);
        let resume_action = gio::SimpleAction::new("resume", None);
        let preferences_action = gio::SimpleAction::new("preferences", None);
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
        let fullscreened = {
            let settings = state.borrow().settings();
            current_or_default(&settings).video.fullscreen
        };
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state: state.clone(),
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
            fullscreened,
        }));
        let _ = GLArea::bind(glarea.clone(), result.state());
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
                    let settings = result.state().borrow().settings();
                    let settings = current_or_default(&settings);
                    if settings.host.clear_input_on_focus_loss {
                        result.state().borrow_mut().clear_controller_input();
                    }
                    if settings.host.pause_on_focus_loss {
                        let _ = result
                            .state()
                            .borrow_mut()
                            .run_command(SessionCommand::Pause);
                        result.update_actions();
                    }
                }
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
                let _ = result
                    .state()
                    .borrow_mut()
                    .run_command(SessionCommand::Pause);
                result.update_actions();
            });
        }
        window.add_action(&pause_action);

        {
            let result = result.clone();
            let _ = resume_action.connect_activate(move |_, _| {
                let _ = result
                    .state()
                    .borrow_mut()
                    .run_command(SessionCommand::Resume);
                result.update_actions();
            });
        }
        window.add_action(&resume_action);

        {
            let result = result.clone();
            let _ = preferences_action.connect_activate(move |_, _| {
                let settings = result.state().borrow().settings();
                present_preferences_dialog(&result.window(), settings);
            });
        }
        window.add_action(&preferences_action);

        {
            let result = result.clone();
            let _ = state_create_action.connect_activate(move |_, _| {
                let _ = result
                    .state()
                    .borrow_mut()
                    .run_command(SessionCommand::CreateSlot);
                result.update_actions();
            });
        }
        window.add_action(&state_create_action);

        {
            let result = result.clone();
            let _ = state_save_active_action.connect_activate(move |_, _| {
                let _ = result
                    .state()
                    .borrow_mut()
                    .run_command(SessionCommand::SaveActiveSlotOrNew);
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
                    let _ = result
                        .state()
                        .borrow_mut()
                        .run_command(SessionCommand::SelectActiveSlot(slot_id));
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_select_slot_action);

        {
            let result = result.clone();
            let _ = state_save_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result
                        .state()
                        .borrow_mut()
                        .run_command(SessionCommand::SaveSlot(slot_id));
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_save_slot_action);

        {
            let result = result.clone();
            let _ = state_load_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result
                        .state()
                        .borrow_mut()
                        .run_command(SessionCommand::LoadSlot(slot_id));
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_load_slot_action);

        {
            let result = result.clone();
            let _ = state_delete_slot_action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result
                        .state()
                        .borrow_mut()
                        .run_command(SessionCommand::DeleteSlot(slot_id));
                    result.update_actions();
                }
            });
        }
        window.add_action(&state_delete_slot_action);

        {
            let result = result.clone();
            let _ = glib::timeout_add_local(TITLE_UPDATE_INTERVAL, move || {
                result.refresh_title();
                glib::ControlFlow::Continue
            });
        }

        result.update_actions();
        window.present();
        let _ = glarea.grab_focus();
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
        let settings = self.state().borrow().settings();
        let settings = current_or_default(&settings);
        if let Some(path) = settings
            .paths
            .default_open_dir
            .or(settings.general.last_open_directory)
        {
            let folder = gio::File::for_path(path);
            let _ = file_chooser_native.set_current_folder(Some(&folder));
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
        if let Some(mut f) = File::open(path).ok().map(BufReader::new) {
            let mut buf = Vec::new();
            let _ = f.read_to_end(&mut buf).unwrap();
            self.state()
                .borrow_mut()
                .load_from_path(Some(path.to_path_buf()), buf);
            self.update_actions();
        }
    }

    fn close(&self) {
        let _ = self.state().borrow_mut().unload();
        self.update_actions();
    }

    fn realize(&self) {}

    fn close_request(&self) -> bool {
        let window = self.window();
        if !self.borrow().fullscreened {
            let width = window.width().max(1) as u32;
            let height = window.height().max(1) as u32;
            let _ = self
                .state()
                .borrow()
                .settings()
                .remember_window_size(width, height);
        }
        self.state().borrow_mut().flush_before_exit();
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

    fn refresh_title(&self) {
        let title = self.state().borrow().title();
        self.window().set_title(Some(title.as_str()));
    }

    fn key_event(&self, key: gdk::Key, event: KeyEventState) -> bool {
        let settings = self.state().borrow().settings();
        let settings = current_or_default(&settings);
        if let Some(key) = gdk_key_settings_key(key) {
            if matches!(event, KeyEventState::Release) {
                if let Some(controller_event) = controller_event_for_key(&settings, key, false) {
                    self.state()
                        .borrow_mut()
                        .handle_controller_input(controller_event);
                }
                if matches!(
                    shortcut_action_for_key(&settings, key),
                    Some(ShortcutAction::ToggleFullscreen)
                ) {
                    let fullscreened = {
                        let mut core = self.borrow_mut();
                        core.fullscreened = !core.fullscreened;
                        core.fullscreened
                    };
                    if fullscreened {
                        self.window().fullscreen();
                    } else {
                        self.window().unfullscreen();
                    }
                    return false;
                }
                if let Some(command) = shortcut_command_for_key(&settings, key) {
                    match command {
                        SessionCommand::TogglePause
                        | SessionCommand::Reset
                        | SessionCommand::SaveActiveSlotOrNew
                        | SessionCommand::LoadActiveSlot
                        | SessionCommand::SelectNextSlot
                        | SessionCommand::SelectPreviousSlot => {
                            let _ = self.state().borrow_mut().run_command(command);
                            self.update_actions();
                            return false;
                        }
                        _ => {}
                    }
                }
                return false;
            }
            if let Some(event) = controller_event_for_key(&settings, key, key_event_pressed(event))
            {
                self.state().borrow_mut().handle_controller_input(event);
            }
        }
        false
    }
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

#[cfg(test)]
mod tests {
    use super::{ActiveSlotLoader, gdk_key_settings_key, load_active_slot};
    use nerust_contract_settings::input::KeyboardKey;
    use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeState {
        active_slot_id: Option<u64>,
        loaded_slot_id: Option<u64>,
    }

    impl ActiveSlotLoader for FakeState {
        fn active_slot_id(&self) -> Option<u64> {
            self.active_slot_id
        }

        fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
            if let SessionCommand::LoadSlot(slot_id) = command {
                self.loaded_slot_id = Some(slot_id);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            } else {
                SessionCommandOutcome::default()
            }
        }
    }

    #[test]
    fn load_active_slot_releases_shared_borrow_before_mutating() {
        let state = RefCell::new(FakeState {
            active_slot_id: Some(8),
            loaded_slot_id: None,
        });

        assert!(load_active_slot(&state));
        assert_eq!(state.borrow().loaded_slot_id, Some(8));
    }

    #[test]
    fn load_active_slot_is_noop_without_selection() {
        let state = RefCell::new(FakeState::default());

        assert!(!load_active_slot(&state));
        assert_eq!(state.borrow().loaded_slot_id, None);
    }

    #[test]
    fn gdk_key_mapping_matches_settings_keys() {
        assert_eq!(gdk_key_settings_key(gdk::Key::z), Some(KeyboardKey::KeyZ));
        assert_eq!(gdk_key_settings_key(gdk::Key::x), Some(KeyboardKey::KeyX));
        assert_eq!(
            gdk_key_settings_key(gdk::Key::Up),
            Some(KeyboardKey::ArrowUp)
        );
        assert_eq!(
            gdk_key_settings_key(gdk::Key::Right),
            Some(KeyboardKey::ArrowRight)
        );
        assert_eq!(
            gdk_key_settings_key(gdk::Key::Return),
            Some(KeyboardKey::Enter)
        );
    }
}
