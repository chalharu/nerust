use super::glarea::{GLArea, GLAreaExtend};
use super::State;
use gio::prelude::*;
use gtk::prelude::*;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::LogicalSize;
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::rc::Rc;

pub struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    // glarea: GLArea,
    state: Rc<RefCell<Option<State>>>,
}

pub type Window = Rc<RefCell<WindowCore>>;

pub trait WindowExtend {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        glarea: gtk::GLArea,
        state: Rc<RefCell<Option<State>>>,
    ) -> Window;
    fn window(&self) -> gtk::ApplicationWindow;
    fn application(&self) -> gtk::Application;
    fn state(&self) -> Rc<RefCell<Option<State>>>;
    fn realize(&self);
    fn delete_event(&self) -> bool;
    fn open(&self);
}

impl WindowExtend for Window {
    fn state(&self) -> Rc<RefCell<Option<State>>> {
        self.borrow().state.clone()
    }

    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        glarea: gtk::GLArea,
        state: Rc<RefCell<Option<State>>>,
    ) -> Window {
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            // glarea: GLArea::bind(glarea, state.clone()),
            state,
        }));
        GLArea::bind(glarea, result.state());
        {
            let result = result.clone();
            window.connect_realize(move |_window| result.realize());
        }
        {
            let result = result.clone();
            window.connect_delete_event(move |_, _| Inhibit(result.delete_event()));
        }
        let open_action = gio::SimpleAction::new("open", None);
        {
            let result = result.clone();
            open_action.connect_activate(move |_, _| {
                result.open();
            });
        }
        window.add_action(&open_action);
        window.show_all();
        result
    }

    fn open(&self) {
        let file_chooser_native = gtk::FileChooserNative::new(
            "Open File",
            Some(&self.window()),
            gtk::FileChooserAction::Open,
            "_Open",
            "_Cancel",
        );
        let state = self.state();
        file_chooser_native.connect_response(move |file_chooser_native, _| {
            if let Some(mut f) = file_chooser_native
                .get_filename()
                .and_then(|f| File::open(f).ok())
                .map(|f| BufReader::new(f))
            {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).unwrap();
                let mut state = state.borrow_mut();
                if let Some(ref mut state) = *state {
                    state.console.load(buf);
                    state.console.resume();
                }
            }
        });
        file_chooser_native.run();
    }

    fn realize(&self) {
        let screen_buffer = ScreenBuffer::new(
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );

        *self.state().borrow_mut() = Some(State::new(screen_buffer));
    }

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
}
