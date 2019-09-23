use super::glarea::{GLArea, GLAreaExtend};
use super::State;
use gio::prelude::*;
use gtk::prelude::*;
use nerust_core::controller::standard_controller::Buttons;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::rc::Rc;

pub(crate) struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    // glarea: GLArea,
    state: Rc<RefCell<State>>,
    keys: Buttons,
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
    fn delete_event(&self) -> bool;
    fn open(&self);
    fn close(&self);
    fn pause(&self);
    fn resume(&self);
    fn key_event(&self, key: &gdk::EventKey, enevt: KeyEventState) -> bool;
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
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            // glarea: GLArea::bind(glarea, state.clone()),
            state: state.clone(),
            keys: Buttons::empty(),
        }));
        let _ = GLArea::bind(glarea, result.state());
        {
            let result = result.clone();
            let _ = window.connect_realize(move |_window| result.realize());
        }
        {
            let result = result.clone();
            let _ = window.connect_delete_event(move |_, _| Inhibit(result.delete_event()));
        }

        {
            let result = result.clone();
            let _ = window.connect_key_press_event(move |_, event_key| {
                Inhibit(result.key_event(event_key, KeyEventState::Press))
            });
        }
        {
            let result = result.clone();
            let _ = window.connect_key_release_event(move |_, event_key| {
                Inhibit(result.key_event(event_key, KeyEventState::Release))
            });
        }
        let open_action = gio::SimpleAction::new("open", None);
        let close_action = gio::SimpleAction::new("close", None);
        let pause_action = gio::SimpleAction::new("pause", None);
        let resume_action = gio::SimpleAction::new("resume", None);

        let update_func = {
            let close_action = close_action.clone();
            let pause_action = pause_action.clone();
            let resume_action = resume_action.clone();
            move || {
                close_action.set_enabled(state.borrow_mut().loaded());
                pause_action.set_enabled(state.borrow_mut().can_pause());
                resume_action.set_enabled(state.borrow_mut().can_resume());
            }
        };

        {
            let result = result.clone();
            let update_func = update_func.clone();
            let _ = open_action.connect_activate(move |_, _| {
                result.open();
                update_func();
            });
        }
        window.add_action(&open_action);

        {
            let result = result.clone();
            let update_func = update_func.clone();
            let _ = close_action.connect_activate(move |_, _| {
                result.close();
                update_func();
            });
        }
        window.add_action(&close_action);

        {
            let result = result.clone();
            let update_func = update_func.clone();
            let _ = pause_action.connect_activate(move |_, _| {
                result.pause();
                update_func();
            });
        }
        window.add_action(&pause_action);

        {
            let result = result.clone();
            let update_func = update_func.clone();
            let _ = resume_action.connect_activate(move |_, _| {
                result.resume();
                update_func();
            });
        }
        window.add_action(&resume_action);

        update_func();
        window.show_all();
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
        let state = self.state();
        let _ = file_chooser_native.connect_response(move |file_chooser_native, _| {
            if let Some(mut f) = file_chooser_native
                .get_filename()
                .and_then(|f| File::open(f).ok())
                .map(BufReader::new)
            {
                let mut buf = Vec::new();
                let _ = f.read_to_end(&mut buf).unwrap();
                state.borrow_mut().load(buf);
            }
        });
        let _ = file_chooser_native.run();
    }

    fn close(&self) {
        self.state().borrow_mut().unload();
    }

    fn pause(&self) {
        self.state().borrow_mut().pause();
    }

    fn resume(&self) {
        self.state().borrow_mut().resume();
    }

    fn realize(&self) {}

    fn delete_event(&self) -> bool {
        self.application().quit();
        false
    }

    fn window(&self) -> gtk::ApplicationWindow {
        self.borrow().window.clone()
    }

    fn application(&self) -> gtk::Application {
        self.borrow().application.clone()
    }

    fn key_event(&self, key: &gdk::EventKey, event: KeyEventState) -> bool {
        // とりあえず、pad1のみ次の通りとする。
        // A      -> Z
        // B      -> X
        // Select -> C
        // Start  -> V
        // Up     -> Up
        // Down   -> Down
        // Left   -> Left
        // Right  -> Right
        let code = match key.get_keyval() {
            gdk::enums::key::z => Buttons::A,
            gdk::enums::key::x => Buttons::B,
            gdk::enums::key::c => Buttons::SELECT,
            gdk::enums::key::v => Buttons::START,
            gdk::enums::key::Up => Buttons::UP,
            gdk::enums::key::Down => Buttons::DOWN,
            gdk::enums::key::Left => Buttons::LEFT,
            gdk::enums::key::Right => Buttons::RIGHT,
            _ => Buttons::empty(),
        };
        let key = self.borrow().keys;
        self.borrow_mut().keys = match event {
            KeyEventState::Press => key | code,
            KeyEventState::Release => key & !code,
        };
        self.state().borrow_mut().set_pad1(self.borrow_mut().keys);
        false
    }
}
