//! `wakui_gtk`: wakui platform frontend for gtk4.
//!
//! Framework-only: depends on [`wakui`] plus gtk4. Never on view definitions,
//! sessions, or settings. Policy flows solely through
//! [`wakui::app::FrontendApp`], injected as a boxed host at [`run`].
//!
//! Native frontends retain [`controls`] dialogs from
//! [`wakui::control::ControlScreen`] snapshots, interpret them through the
//! retained [`renderer`], and map native input through [`keyboard`].
//!
//! Dialog lifecycle helpers live in [`dialog`]. Modal presentation stays
//! native (`native::dialog`); the host retains the queue, pause ownership,
//! and stale-ACK checks.

pub mod backend;
pub mod controls;
pub mod dialog;
pub mod keyboard;

mod gdk_raw;
mod message_dialog;
mod native;
mod surface;
mod surface_state;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
pub(crate) mod testkit;
mod window;

pub use backend::{GtkBackend, register};
pub use message_dialog::show_message_dialog;

use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

use gtk::{
    gdk::prelude::SurfaceExt as _,
    gio, glib,
    prelude::{
        ApplicationExt as _, ApplicationExtManual as _, FileExt as _, GtkApplicationExt as _,
        GtkWindowExt as _, NativeExt as _, WidgetExt as _,
    },
};
use wakui::bootstrap::Application;

use self::window::{MenubarModels, Window, WindowExtend};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct State<K: Copy + Eq + std::hash::Hash + 'static> {
    pub(crate) host: Box<dyn wakui::app::FrontendApp<K>>,
    renderer_reload_pending: bool,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> State<K> {
    pub(crate) fn new(app: Application<K>) -> Self {
        Self {
            host: app.into_app(),
            renderer_reload_pending: false,
        }
    }

    pub(crate) fn swap_frame_buffer(&mut self) {
        self.host.swap_frame_buffer();
    }

    /// Load a ROM, flagging a renderer rebuild on success.
    /// Returns the failure detail; presenting it is the caller's job.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        if let Err(error) = self.host.load_rom(path) {
            log::warn!("ROM load failed: {error}");
            return Err(error);
        }
        if self.host.tick_loaded() {
            self.renderer_reload_pending = true;
        }
        Ok(())
    }

    pub(crate) fn title_update(&mut self) -> Option<String> {
        self.host.title_update()
    }

    pub(crate) fn title_refresh_due(&mut self, now: std::time::Instant) -> bool {
        self.host.title_refresh_due(now)
    }

    /// Ignore intermediate native observations while awaiting the target.
    pub(crate) fn observe_fullscreen(&mut self, native: bool) -> bool {
        self.host.observe_fullscreen(native)
    }

    /// Ordered teardown entry: false when a close is already in flight.
    pub(crate) fn begin_close(&mut self) -> bool {
        self.host.begin_close()
    }

    pub(crate) fn finish_close(&mut self) {
        self.host.finish_close();
    }
}

#[cfg(test)]
impl State<crate::testkit::TestAux> {
    /// Test-only construction over the headless [`testkit`] app.
    pub(crate) fn new_for_tests() -> Self {
        Self::new(Application::new(Box::new(crate::testkit::FakeApp::new())))
    }
}

impl<K: Copy + Eq + std::hash::Hash + 'static> State<K> {
    pub(crate) fn tick_focus_sync(&mut self) {
        use wakui::focus::FocusEvent;
        let _ = self.host.handle_event(
            wakui::event::EventCommand::Focus(FocusEvent::BurstEnd),
            false,
        );
    }
}

fn build_window<K: Copy + Eq + std::hash::Hash + 'static>(
    app: &gtk::Application,
    state: Rc<RefCell<State<K>>>,
) -> Window<K> {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menubar = MenubarModels::new();

    app.add_window(&window);
    // Game viewport: a memory picture composited by GTK; backends never present here.
    let picture = gtk::Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_shrink(true);

    Window::bind(app.clone(), window, picture, state, menubar)
}

fn ensure_window<K: Copy + Eq + std::hash::Hash + 'static>(
    app: &gtk::Application,
    state: &Rc<RefCell<State<K>>>,
    current_window: &Rc<RefCell<Option<Window<K>>>>,
) -> Window<K> {
    if let Some(window) = current_window.borrow().as_ref().cloned() {
        return window;
    }

    let window = build_window(app, Rc::clone(state));
    *current_window.borrow_mut() = Some(window.clone());
    window
}

/// Exit status for the binary wrapper: the library never terminates the process.
/// Crash containment and boot failure dialogs live in the composition root;
/// see `nerust::run`.
///
/// The launch document (if any) travels inside `app`: it was set by the
/// composition root through [`Application::builder`], so `run` takes a single
/// bundle and every backend honors it the same way.
pub fn run<K: Copy + Eq + std::hash::Hash + 'static>(app: Application<K>) -> i32 {
    run_inner(app);
    0
}

fn run_inner<K: Copy + Eq + std::hash::Hash + 'static>(mut app: Application<K>) {
    // Drained once at startup: `activate` fires only when no `open` event
    // carries files, so the launch document loads exactly once. The `open`
    // handler drops it (OS-delivered files win there) to avoid double loads
    // of the same CLI path forwarded by the toolkit.
    let initial_document = Rc::new(RefCell::new(app.take_initial_document()));
    let gtk_app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    let current_window = Rc::new(RefCell::new(None));
    let state = Rc::new(RefCell::new(State::new(app)));
    {
        let state = Rc::clone(&state);
        let current_window = current_window.clone();
        let initial_document = Rc::clone(&initial_document);
        let _ = gtk_app.connect_activate(move |app| {
            let window = ensure_window(app, &state, &current_window);
            window.window().present();
            if let Some(path) = initial_document.borrow_mut().take() {
                window.load_path(&path);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let current_window = current_window.clone();
        let initial_document = Rc::clone(&initial_document);
        let _ = gtk_app.connect_open(move |app, files, _| {
            let window = ensure_window(app, &state, &current_window);
            // Present first: a failure dialog raised by load_path must
            // stay above the main window.
            window.window().present();
            let _ = initial_document.borrow_mut().take();
            if let Some(path) = files.iter().find_map(|file| file.path()) {
                // One path for every open: failure dialog and action
                // sync live in `Window::load_path`.
                window.load_path(&path);
            }
        });
    }

    // External destruction bypasses close-request: poll the GDK flag so the app can quit.
    {
        let gtk_app = gtk_app.clone();
        let current_window = current_window.clone();
        glib::timeout_add_seconds_local(1, move || {
            let destroyed = current_window
                .borrow()
                .as_ref()
                .and_then(|window| window.window().surface())
                .is_some_and(|surface| surface.is_destroyed());
            if destroyed {
                log::warn!("gtk: native surface destroyed externally; quitting");
                gtk_app.quit();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    let _ = gtk_app.run();
}
