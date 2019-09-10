#[macro_use]
extern crate log;

mod glarea;
mod window;

use self::window::{Window, WindowExtend};
use gio::prelude::*;
use gtk::prelude::*;
use nerust_console::Console;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use std::cell::RefCell;
use std::rc::Rc;

pub struct State {
    view: Option<GlView>,
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl State {
    pub fn new(screen_buffer: ScreenBuffer) -> Self {
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

    pub fn pause(&mut self) {
        self.console.pause();
        self.paused = true;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn can_pause(&self) -> bool {
        !self.paused && self.loaded
    }

    pub fn resume(&mut self) {
        self.console.resume();
        self.paused = false;
    }

    pub fn can_resume(&self) -> bool {
        self.paused && self.loaded
    }

    pub fn load(&mut self, data: Vec<u8>) {
        self.console.load(data);
        self.loaded = true;
        self.resume();
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn unload(&mut self) {
        self.console.unload();
        self.loaded = false;
    }

    pub fn set_pad1(&mut self, data: Buttons) {
        self.console.set_pad1(data)
    }
}

fn app_activate(app: &gtk::Application) {
    let builder = gtk::Builder::new_from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.get_object("window").unwrap();

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new(ScreenBuffer::new(
        FilterType::NtscComposite,
        LogicalSize {
            width: 256,
            height: 240,
        },
    ))));

    app.set_menubar(
        gtk::Builder::new_from_string(include_str!("../resources/menu.xml"))
            .get_object::<gio::Menu>("menu")
            .as_ref(),
    );
    app.add_window(&window);

    let quit_action = gio::SimpleAction::new("quit", None);
    {
        let app = app.clone();
        quit_action.connect_activate(move |_, _| {
            app.quit();
        });
    }
    app.add_action(&quit_action);

    fn create_about_dialog() -> Option<gtk::AboutDialog> {
        Some(
            gtk::Builder::new_from_string(include_str!("../resources/about.xml"))
                .get_object("about")
                .unwrap(),
        )
    }
    let about_action = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        let window_about: Rc<RefCell<Option<gtk::AboutDialog>>> =
            Rc::new(RefCell::new(create_about_dialog()));
        about_action.connect_activate(move |_, _| {
            let window_about_inner = std::mem::replace(&mut *window_about.borrow_mut(), None);
            if let Some(window_about_inner) = window_about_inner {
                window_about_inner.set_transient_for(Some(&window));
                window_about_inner.run();
                window_about_inner.destroy();
                *window_about.borrow_mut() = create_about_dialog();
            }
        });
    }
    app.add_action(&about_action);

    Window::bind(
        app.clone(),
        window,
        builder.get_object("glarea").unwrap(),
        state,
    );
}

fn main() {
    // log initialize
    simple_logger::init().unwrap();

    let app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    )
    .expect("Application start up error");

    app.connect_activate(app_activate);

    app.run(&std::env::args().collect::<Vec<_>>());
}
