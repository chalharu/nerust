mod crash_handler;
mod glarea;
mod window;

use self::window::{StateMenus, Window, WindowExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_backend_opengl::GlBackend;
use nerust_gui_runtime::{
    ConsoleSessionFactory, ConsoleVideo, ControllerInput, ControllerPort, GuiSession, InputState,
    SessionCommand, SessionCommandOutcome, StateSlotSummary, WindowSize,
};
use nerust_gui_shell::{NesConsoleDescriptor, NesInputAdapter};
use nerust_sound_openal::prepare_macos_runtime;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub(crate) struct State {
    view: Option<GlBackend>,
    session: GuiSession,
    input: NesInputAdapter,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            view: None,
            session: NesConsoleDescriptor.build_session(),
            input: NesInputAdapter::new(),
        }
    }

    pub(crate) fn video(&self) -> &ConsoleVideo {
        self.session.video()
    }

    pub(crate) fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.session.with_frame_buffer(f)
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
        if self.session.load(rom_path, data) {
            self.input.clear(&mut self.session);
            let _ = self.session.run_command(SessionCommand::Resume);
        }
    }

    pub(crate) fn loaded(&self) -> bool {
        self.session.loaded()
    }

    pub(crate) fn title(&self) -> String {
        self.session.window_title()
    }

    pub(crate) fn unload(&mut self) -> bool {
        let unloaded = self.session.unload();
        if unloaded {
            self.input.clear(&mut self.session);
        }
        unloaded
    }

    pub(crate) fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub(crate) fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        self.session.run_command(command)
    }

    pub(crate) fn handle_controller_input(
        &mut self,
        port: ControllerPort,
        input: ControllerInput,
        state: InputState,
    ) {
        self.input.handle_input(port, input, state);
        self.input.flush_to_session(&mut self.session);
    }

    pub(crate) fn clear_controller_input(&mut self) {
        self.input.clear(&mut self.session);
    }

    pub(crate) fn slots(&self) -> &[StateSlotSummary] {
        self.session.slots()
    }

    pub(crate) fn active_slot_id(&self) -> Option<u64> {
        self.session.active_slot_id()
    }
}

fn build_window(app: &gtk::Application) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menu_model = gtk::Builder::from_string(include_str!("../resources/menu.xml"))
        .object::<gio::Menu>("menu")
        .unwrap();
    let state_menu = gio::Menu::new();
    let select_active_slot_menu = gio::Menu::new();
    let save_slot_menu = gio::Menu::new();
    let load_slot_menu = gio::Menu::new();
    let delete_slot_menu = gio::Menu::new();
    state_menu.append(Some("Create Save Slot"), Some("win.state-create"));
    state_menu.append(Some("Save Active Slot"), Some("win.state-save-active"));
    state_menu.append(Some("Load Active Slot"), Some("win.state-load-active"));
    state_menu.append_submenu(Some("Select Active Slot"), &select_active_slot_menu);
    state_menu.append_submenu(Some("Save Slot"), &save_slot_menu);
    state_menu.append_submenu(Some("Load Slot"), &load_slot_menu);
    state_menu.append_submenu(Some("Delete Slot"), &delete_slot_menu);
    menu_model.append_submenu(Some("Save States"), &state_menu);

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new()));

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
