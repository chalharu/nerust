mod crash_handler;
mod glarea;
mod window;

use self::window::{Window, WindowExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_console::{Console, ConsoleMetrics};
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::{OpenAl, prepare_macos_runtime};
use nerust_timer::CLOCK_RATE;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

fn window_title(paused: bool, console_metrics: ConsoleMetrics) -> String {
    let state = if paused { "Nes -- Paused" } else { "Nes" };
    if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
    }
}

#[derive(Debug)]
pub(crate) struct State {
    view: Option<GlView>,
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
}

impl State {
    pub(crate) fn new(filter_type: FilterType, source_logical_size: LogicalSize) -> Self {
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new_gpu(speaker, filter_type, source_logical_size);
        let physical_size = console.video().presentation().physical_size();
        Self {
            view: None,
            console,
            paused: false,
            loaded: false,
            physical_size,
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

    pub(crate) fn title(&self) -> String {
        window_title(self.paused, self.console.metrics())
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

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new(
        FilterType::NtscComposite,
        LogicalSize {
            width: 256,
            height: 240,
        },
    )));

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

#[cfg(test)]
mod tests {
    use super::window_title;
    use nerust_console::ConsoleMetrics;

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }
}
