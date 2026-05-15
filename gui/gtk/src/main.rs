mod crash_handler;
mod glarea;
mod window;

use self::window::{Window, WindowExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_console::Console;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::{OpenAl, prepare_macos_runtime};
use nerust_timer::CLOCK_RATE;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug)]
pub(crate) struct State {
    view: Option<GlView>,
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl State {
    pub(crate) fn new(screen_buffer: ScreenBuffer) -> Self {
        let physical_size = screen_buffer.physical_size();
        let logical_size = screen_buffer.logical_size();
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new(speaker, screen_buffer);
        Self {
            view: None,
            console,
            paused: false,
            loaded: false,
            physical_size,
            logical_size,
        }
    }

    pub(crate) fn pause(&mut self) {
        self.console.pause();
        self.paused = true;
    }

    #[allow(dead_code, reason = "reserved for GTK menu state bindings")]
    pub(crate) fn paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn can_pause(&self) -> bool {
        !self.paused && self.loaded
    }

    pub(crate) fn resume(&mut self) {
        self.console.resume();
        self.paused = false;
    }

    pub(crate) fn can_resume(&self) -> bool {
        self.paused && self.loaded
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        self.console.load(data);
        self.loaded = true;
        self.resume();
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn unload(&mut self) {
        self.console.unload();
        self.loaded = false;
    }

    pub(crate) fn set_pad1(&mut self, data: Buttons) {
        self.console.set_pad1(data)
    }
}

fn build_window(app: &gtk::Application) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menu = gtk::Builder::from_string(include_str!("../resources/menu.xml"))
        .object::<gio::Menu>("menu")
        .unwrap();

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new(ScreenBuffer::new(
        FilterType::NtscComposite,
        LogicalSize {
            width: 256,
            height: 240,
        },
    ))));

    app.set_menubar(Some(&menu));
    app.add_window(&window);
    window.set_show_menubar(true);

    let quit_action = gio::SimpleAction::new("quit", None);
    {
        let app = app.clone();
        let _ = quit_action.connect_activate(move |_, _| {
            app.quit();
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
