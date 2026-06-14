mod crash_handler;
mod glarea;
mod preferences;
mod renderer;
mod window;

use self::window::{StateMenus, Window, WindowExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsApplyPlan, SettingsSnapshot};
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_shell::descriptor::SystemSettingsPageModel;
use nerust_gui_shell::load::{LoadRequest, MediaObject};
use nerust_gui_shell::session::{KeyboardShortcut, SessionHandle, SessionSnapshot};
use nerust_gui_shell::settings::i18n::{UiText, text};
use nerust_persistence::model::StateSlotSummary;
use nerust_sound_openal::prepare_macos_runtime;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct State {
    session: SessionHandle,
    renderer_reload_pending: bool,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            session: SessionHandle::new_for_host(HostBackendIdentity::gtk_opengl()),
            renderer_reload_pending: false,
        }
    }

    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        self.session.snapshot()
    }

    pub(crate) fn system_settings_page_model(&self) -> SystemSettingsPageModel {
        self.session.system_settings_page_model()
    }

    pub(crate) fn input_topology_descriptor(&self) -> nerust_input_schema::InputTopologyDescriptor {
        self.session.input_topology_descriptor()
    }

    pub(crate) fn window_size(&self) -> WindowSize {
        self.session.window_size()
    }

    pub(crate) fn can_pause(&self) -> bool {
        self.session.can_pause()
    }

    pub(crate) fn can_resume(&self) -> bool {
        self.session.can_resume()
    }

    pub(crate) fn load_from_path(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) {
        if self
            .session
            .load(MediaObject::new(rom_path, data), LoadRequest::Auto)
            .is_ok()
        {
            let _ = self.session.run_command(SessionCommand::Resume);
        }
    }

    pub(crate) fn loaded(&self) -> bool {
        self.session.loaded()
    }

    pub(crate) fn paused(&self) -> bool {
        self.session.paused()
    }

    pub(crate) fn title(&self) -> String {
        self.session.window_title()
    }

    pub(crate) fn unload(&mut self) -> bool {
        self.session.unload().unwrap_or(false)
    }

    pub(crate) fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub(crate) fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        self.session.run_command(command).unwrap_or_default()
    }

    pub(crate) fn handle_keyboard_key(
        &mut self,
        key: KeyboardKey,
        pressed: bool,
    ) -> Option<KeyboardShortcut> {
        self.session
            .handle_keyboard_key(key, pressed)
            .ok()
            .flatten()
    }

    pub(crate) fn clear_input(&mut self) {
        let _ = self.session.clear_input();
    }

    pub(crate) fn slots(&self) -> &[StateSlotSummary] {
        self.session.slots()
    }

    pub(crate) fn active_slot_id(&self) -> Option<u64> {
        self.session.active_slot_id()
    }

    pub(crate) fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.session.settings_snapshot()
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, String> {
        let plan = self.session.apply_settings(settings)?;
        if plan.session_rebuild_required || plan.window_settings_changed {
            self.renderer_reload_pending = true;
        }
        Ok(plan)
    }

    pub(crate) fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsApplyPlan, String> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        if plan.session_rebuild_required || plan.window_settings_changed {
            self.renderer_reload_pending = true;
        }
        Ok(plan)
    }

    pub(crate) fn take_renderer_reload_pending(&mut self) -> bool {
        std::mem::take(&mut self.renderer_reload_pending)
    }
}

fn build_window(app: &gtk::Application) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let state_menu = gio::Menu::new();
    let select_active_slot_menu = gio::Menu::new();
    let save_slot_menu = gio::Menu::new();
    let load_slot_menu = gio::Menu::new();
    let delete_slot_menu = gio::Menu::new();

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new()));
    let language = state.borrow().settings_snapshot().shared.general.language;
    let menu_model = build_menu_model(
        language,
        &state_menu,
        &select_active_slot_menu,
        &save_slot_menu,
        &load_slot_menu,
        &delete_slot_menu,
    );

    app.set_menubar(Some(&menu_model));
    app.add_window(&window);
    window.set_show_menubar(true);

    let quit_action = gio::SimpleAction::new("quit", None);
    {
        let window = window.clone();
        let _ = quit_action.connect_activate(move |_, _| {
            window.close();
        });
    }
    app.add_action(&quit_action);

    fn create_about_dialog() -> gtk::AboutDialog {
        gtk::Builder::from_string(include_str!("../resources/about.xml"))
            .object("about")
            .unwrap()
    }
    let about_action = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        let window_about: Rc<RefCell<Option<gtk::AboutDialog>>> = Rc::new(RefCell::new(None));
        let _ = about_action.connect_activate(move |_, _| {
            if let Some(dialog) = window_about.borrow().as_ref() {
                dialog.present();
                return;
            }

            let dialog = create_about_dialog();
            dialog.set_transient_for(Some(&window));

            let window_about_on_close = window_about.clone();
            let _ = dialog.connect_close_request(move |_| {
                *window_about_on_close.borrow_mut() = None;
                glib::Propagation::Proceed
            });

            dialog.present();
            *window_about.borrow_mut() = Some(dialog);
        });
    }
    app.add_action(&about_action);

    Window::bind(
        app.clone(),
        window,
        builder.object("glarea").unwrap(),
        state,
        StateMenus {
            select_active_slot_menu,
            save_slot_menu,
            load_slot_menu,
            delete_slot_menu,
        },
    )
}

pub(crate) fn build_menu_model(
    language: AppLanguage,
    state_menu: &gio::Menu,
    select_active_slot_menu: &gio::Menu,
    save_slot_menu: &gio::Menu,
    load_slot_menu: &gio::Menu,
    delete_slot_menu: &gio::Menu,
) -> gio::Menu {
    state_menu.remove_all();
    state_menu.append(
        Some(text(language, UiText::CreateSaveSlot)),
        Some("win.state-create"),
    );
    state_menu.append(
        Some(text(language, UiText::SaveActiveSlot)),
        Some("win.state-save-active"),
    );
    state_menu.append(
        Some(text(language, UiText::LoadActiveSlot)),
        Some("win.state-load-active"),
    );
    state_menu.append_submenu(
        Some(text(language, UiText::SelectActiveSlot)),
        select_active_slot_menu,
    );
    state_menu.append_submenu(Some(text(language, UiText::SaveSlot)), save_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::LoadSlot)), load_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::DeleteSlot)), delete_slot_menu);

    let file_menu = gio::Menu::new();
    file_menu.append(Some(text(language, UiText::Open)), Some("win.open"));
    file_menu.append(Some(text(language, UiText::Close)), Some("win.close"));
    file_menu.append(Some(text(language, UiText::Settings)), Some("win.settings"));
    file_menu.append(Some(text(language, UiText::Quit)), Some("app.quit"));

    let emulation_menu = gio::Menu::new();
    emulation_menu.append(Some(text(language, UiText::Pause)), Some("win.pause"));
    emulation_menu.append(Some(text(language, UiText::Resume)), Some("win.resume"));
    emulation_menu.append_submenu(Some(text(language, UiText::SaveStates)), state_menu);

    let help_menu = gio::Menu::new();
    help_menu.append(Some(text(language, UiText::About)), Some("app.about"));

    let menu = gio::Menu::new();
    menu.append_submenu(Some(text(language, UiText::File)), &file_menu);
    menu.append_submenu(Some(text(language, UiText::Emulation)), &emulation_menu);
    menu.append_submenu(Some(text(language, UiText::About)), &help_menu);
    menu
}

fn ensure_window(app: &gtk::Application, current_window: &Rc<RefCell<Option<Window>>>) -> Window {
    if let Some(window) = current_window.borrow().as_ref().cloned() {
        return window;
    }

    let window = build_window(app);
    *current_window.borrow_mut() = Some(window.clone());
    window
}

fn main() {
    // log initialize
    simple_logger::init().unwrap();
    crash_handler::install();
    prepare_macos_runtime();

    let app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    let current_window = Rc::new(RefCell::new(None));
    {
        let current_window = current_window.clone();
        let _ = app.connect_activate(move |app| {
            let window = ensure_window(app, &current_window);
            window.window().present();
        });
    }
    {
        let current_window = current_window.clone();
        let _ = app.connect_open(move |app, files, _| {
            let window = ensure_window(app, &current_window);
            if let Some(path) = files.iter().find_map(|file| file.path()) {
                window.load_path(&path);
            }
            window.window().present();
        });
    }

    let _ = app.run();
}
