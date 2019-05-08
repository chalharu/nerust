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

struct State {
    view: ManuallyDrop<GlView>,
    running: bool,
    keys: Buttons,
    paused: bool,
    console: Console,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.view);
        }
    }
}

impl State {
    pub fn new(screen_buffer: ScreenBuffer) -> Self {
        let physical_size = screen_buffer.physical_size();
        let logical_size = screen_buffer.logical_size();
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new(speaker, screen_buffer);
        let mut view = GlView::new();
        view.use_vao(true);
        Self {
            view: ManuallyDrop::new(view),
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

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_position(gtk::WindowPosition::Center);

    window.set_title("Nes");

    window.set_default_size(1000, 800);

    let gl_area = gtk::GLArea::new();
    gl_area.set_vexpand(true);
    gl_area.set_halign(gtk::Align::Fill);
    gl_area.set_vexpand(true);
    gl_area.set_valign(gtk::Align::Fill);
    gl_area.set_required_version(3, 2);
    // gl_area.set_required_version(2, 0);
    // gl_area.set_use_es(true);
    gl_area.set_auto_render(true);

    let state: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));

    {
        let state = state.clone();
        // FnOnceではなく、Fnを要求するため、moveoutは不可
        gl_area.connect_realize(move |gl_area| {
            let rom_data =
                include_bytes!(concat!("../../../roms/", "tests/Lan Master/Lan_Master.nes",))
                    .to_vec();

            let screen_buffer = ScreenBuffer::new(
                FilterType::NtscComposite,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            );

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

            let mut state = state.borrow_mut();
            *state = Some(State::new(screen_buffer));
            if let Some(ref mut state) = *state {
                state.console.load(rom_data);
                state.console.resume();
                state.view.on_load(state.logical_size);
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
                // let dpi_factor = glarea.get_scale_factor();

                let rate_x = f64::from(w) / f64::from(state.physical_size.width);
                let rate_y = f64::from(h) / f64::from(state.physical_size.height);
                let rate = f64::min(rate_x, rate_y);
                let scale_x = (rate / rate_x) as f32;
                let scale_y = (rate / rate_y) as f32;

                // self.context.resize(logical_size.to_physical(dpi_factor));
                state.view.on_resize(scale_x, scale_y);
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
                state.view.on_close();
            }
            *state = None;
        });
    }

    {
        window.connect_delete_event(move |win, _| {
            win.destroy();
            gtk::main_quit();
            Inhibit(false)
        });
    }

    gl_area.set_can_focus(true);
    gl_area.grab_focus();

    gl_area.add_tick_callback(move |gl_area, _frame_clock| {
        render(gl_area.downcast_ref().unwrap(), &state);
        true
    });

    window.add(&gl_area);
    window.show_all();

    gtk::main();
}

fn render(gl_area: &gtk::GLArea, state: &Rc<RefCell<Option<State>>>) {
    gl_area.make_current();
    if let Some(e) = gl_area.get_error() {
        error!("{}", e);
    }
    if let Some(ref mut state) = *state.borrow_mut() {
        state
            .view
            .on_update(state.console.logical_size(), state.console.as_ptr());
    }
    unsafe {
        epoxy::Flush();
    }
    gl_area.queue_render();
}
