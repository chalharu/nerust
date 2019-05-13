#[macro_use]
extern crate log;

use gtk::prelude::*;
use nerust_console::Console;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use shared_library::dynamic_library::DynamicLibrary;
use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::ptr;
use std::rc::Rc;
use std::fs::File;
use std::io::{BufReader, Read};

struct State {
    view: Option<GlView>,
    running: bool,
    keys: Buttons,
    paused: bool,
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
            running: true,
            keys: Buttons::empty(),
            paused: false,
            physical_size,
            logical_size,
        }
    }
}

fn main() {
    gtk::init().expect("Failed to initialize GTK.");

    // log initialize
    simple_logger::init().unwrap();

    let ui = include_str!("../resources/window.glade");
    let builder = gtk::Builder::new_from_string(ui);

    let window : gtk::Window = builder.get_object("window").unwrap();
    let state: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));
    let menu_quit : gtk::MenuItem = builder.get_object("menu-quit").unwrap();
    let menu_open : gtk::MenuItem = builder.get_object("menu-open").unwrap();

    menu_quit.connect_activate(move |_| {
        gtk::main_quit();
    });

    window.connect_delete_event(move |_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    {
        let state = state.clone();
        let window = window.clone();
        menu_open.connect_activate(move |_| {
            let state = state.clone();
            let file_chooser_native = gtk::FileChooserNative::new("Open File", Some(&window), gtk::FileChooserAction::Open, "_Open", "_Cancel");
            file_chooser_native.connect_response(move |file_chooser_native, _| {
                if let Some(mut f) = file_chooser_native.get_filename().and_then(|f| File::open(f).ok()).map(|f| BufReader::new(f)) {
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
        });
    }

    {
        let state = state.clone();
        // FnOnceではなく、Fnを要求するため、moveoutは不可
        window.connect_realize(move |_window| {
            let screen_buffer = ScreenBuffer::new(
                FilterType::NtscComposite,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            );

            let mut state = state.borrow_mut();
            *state = Some(State::new(screen_buffer));
        });
    }

    let gl_area : gtk::GLArea = builder.get_object("glarea").unwrap();
    {
        let state = state.clone();
        gl_area.connect_realize(move |gl_area| {
            let mut view = GlView::new();
            view.use_vao(true);
            gl_area.make_current();
            if let Some(e) = gl_area.get_error() {
                error!("{}", e);
            }
            epoxy::load_with(|s| unsafe {
                match DynamicLibrary::open(None).unwrap().symbol(s) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("{}", e);
                        ptr::null()
                    }
                }
            });
            GlView::load_with(epoxy::get_proc_addr);
            if let Some(ref mut state) = *state.borrow_mut() {
                view.on_load(state.logical_size);
                state.view = Some(view);
            }
        });
    }

    {
        let state = state.clone();
        gl_area.connect_resize(move |gl_area, w, h| {
            gl_area.make_current();
            if let Some(e) = gl_area.get_error() {
                error!("{}", e);
            }
            // unsafe {epoxy::Viewport(0, 0, w, h);}
            if let Some(ref mut state) = *state.borrow_mut() {
                let dpi_factor = gl_area.get_scale_factor();

                let rate_x = f64::from(w) / f64::from(state.physical_size.width);
                let rate_y = f64::from(h) / f64::from(state.physical_size.height);
                let rate = f64::min(rate_x, rate_y);
                let scale_x = (rate / rate_x) as f32;
                let scale_y = (rate / rate_y) as f32;

                // self.context.resize(logical_size.to_physical(dpi_factor));
                unsafe {epoxy::Viewport(0, 0, w * dpi_factor, h * dpi_factor);}
                if let Some(ref mut view) = state.view {
                    view.on_resize(scale_x, scale_y);
                }
            }
        });
    }

    {
        let state = state.clone();
        gl_area.connect_render(move |gl_area, _context| {
            render(gl_area, &state);
            Inhibit(true)
        });
    }

    {
        let state = state.clone();
        gl_area.connect_unrealize(move |_gl_area| {
            let mut state = state.borrow_mut();
            if let Some(ref mut state) = *state {
                if let Some(ref mut view) = state.view {
                    view.on_close();
                }
                state.view = None;
            }
            // *state = None;
        });
    }

    {
        let state = state.clone();
        gl_area.add_tick_callback(move |gl_area, _frame_clock| {
            render(gl_area.downcast_ref().unwrap(), &state);
            true
        });
    }

    window.show_all();

    gtk::main();
}


fn render(gl_area: &gtk::GLArea, state: &Rc<RefCell<Option<State>>>) {
    gl_area.make_current();
    if let Some(e) = gl_area.get_error() {
        error!("{}", e);
    }
    if let Some(ref mut state) = *state.borrow_mut() {
        if let Some(ref mut view) = state.view {
            view
            .on_update(state.console.logical_size(), state.console.as_ptr());
        }
    }
    unsafe {
        epoxy::Flush();
    }
    gl_area.queue_render();
}
