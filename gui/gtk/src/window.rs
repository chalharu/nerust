use super::State;
use super::glarea::{GLArea, GLAreaExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_core::controller::standard_controller::Buttons;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::rc::Rc;

pub(crate) struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    keys: Buttons,
    open_dialog: Option<gtk::FileChooserNative>,
    close_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    resume_action: gio::SimpleAction,
}

pub(crate) type Window = Rc<RefCell<WindowCore>>;

pub(crate) trait WindowExtend {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        glarea: gtk::GLArea,
        state: Rc<RefCell<State>>,
    ) -> Window;
    fn window(&self) -> gtk::ApplicationWindow;
    fn application(&self) -> gtk::Application;
    fn state(&self) -> Rc<RefCell<State>>;
    fn realize(&self);
    fn close_request(&self) -> bool;
    fn open(&self);
    fn close(&self);
    fn pause(&self);
    fn resume(&self);
    fn update_actions(&self);
    fn key_event(&self, key: gdk::Key, enevt: KeyEventState) -> bool;
}

pub(crate) enum KeyEventState {
    Press,
    Release,
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
    ) -> Window {
        let close_action = gio::SimpleAction::new("close", None);
        let pause_action = gio::SimpleAction::new("pause", None);
        let resume_action = gio::SimpleAction::new("resume", None);
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state: state.clone(),
            keys: Buttons::empty(),
            open_dialog: None,
            close_action: close_action.clone(),
            pause_action: pause_action.clone(),
            resume_action: resume_action.clone(),
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
                result.pause();
            });
        }
        window.add_action(&pause_action);

        {
            let result = result.clone();
            let _ = resume_action.connect_activate(move |_, _| {
                result.resume();
            });
        }
        window.add_action(&resume_action);

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
        let result = self.clone();
        let _ = file_chooser_native.connect_response(move |file_chooser_native, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(mut f) = file_chooser_native
                    .file()
                    .and_then(|f| f.path())
                    .and_then(|f| File::open(f).ok())
                    .map(BufReader::new)
                {
                    let mut buf = Vec::new();
                    let _ = f.read_to_end(&mut buf).unwrap();
                    result.state().borrow_mut().load(buf);
                }
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

    fn close(&self) {
        self.state().borrow_mut().unload();
        self.update_actions();
    }

    fn pause(&self) {
        self.state().borrow_mut().pause();
        self.update_actions();
    }

    fn resume(&self) {
        self.state().borrow_mut().resume();
        self.update_actions();
    }

    fn realize(&self) {}

    fn close_request(&self) -> bool {
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
        self.borrow()
            .close_action
            .set_enabled(self.state().borrow().loaded());
        self.borrow()
            .pause_action
            .set_enabled(self.state().borrow().can_pause());
        self.borrow()
            .resume_action
            .set_enabled(self.state().borrow().can_resume());
    }

    fn key_event(&self, key: gdk::Key, event: KeyEventState) -> bool {
        // とりあえず、pad1のみ次の通りとする。
        // A      -> Z
        // B      -> X
        // Select -> C
        // Start  -> V
        // Up     -> Up
        // Down   -> Down
        // Left   -> Left
        // Right  -> Right
        let code = match key {
            gdk::Key::z => Buttons::A,
            gdk::Key::x => Buttons::B,
            gdk::Key::c => Buttons::SELECT,
            gdk::Key::v => Buttons::START,
            gdk::Key::Up => Buttons::UP,
            gdk::Key::Down => Buttons::DOWN,
            gdk::Key::Left => Buttons::LEFT,
            gdk::Key::Right => Buttons::RIGHT,
            _ => Buttons::empty(),
        };
        let key = self.borrow().keys;
        self.borrow_mut().keys = match event {
            KeyEventState::Press => key | code,
            KeyEventState::Release => key & !code,
        };
        self.state().borrow_mut().set_pad1(self.borrow().keys);
        false
    }
}
