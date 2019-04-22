use gtk::prelude::*;
use nerust_screen_opengl::GlView;
use shared_library::dynamic_library::DynamicLibrary;
use std::cell::Cell;
use std::path::Path;
use std::ptr;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_core::Core;
use crc::crc64;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::{Timer, CLOCK_RATE};
use std::hash::{Hash, Hasher};
use std::{f64, mem};

#[macro_use]
extern crate log;

type LibPtr = *const std::os::raw::c_void;

trait ProcLoader {
    fn get_proc_addr(&self, s: &str) -> Option<LibPtr>;
}

struct DlProcLoader {
    lib: Option<shared_library::dynamic_library::DynamicLibrary>,
}

fn fn_from<P>(loader: P) -> impl Fn(&str) -> LibPtr
where
    P: ProcLoader + Sized,
{
    move |s| loader.get_proc_addr(s).unwrap_or_else(|| ptr::null())
}

impl DlProcLoader {
    pub fn open(lib_path: &Path) -> Self {
        DlProcLoader {
            lib: DynamicLibrary::open(Some(lib_path)).ok(),
        }
    }
    pub fn current_module() -> Self {
        DlProcLoader {
            lib: DynamicLibrary::open(None).ok(),
        }
    }
}
impl ProcLoader for DlProcLoader {
    fn get_proc_addr(&self, s: &str) -> Option<LibPtr> {
        self.lib
            .as_ref()
            .and_then(|l| match unsafe { l.symbol(s) } {
                Ok(v) => Some(v as LibPtr),
                Err(_) => None,
            })
    }
}

struct Failover<A, B>(pub A, pub B)
where
    A: ProcLoader,
    B: ProcLoader;

impl<A, B> ProcLoader for Failover<A, B>
where
    A: ProcLoader,
    B: ProcLoader,
{
    fn get_proc_addr(&self, s: &str) -> Option<LibPtr> {
        self.0.get_proc_addr(s).or_else(|| self.1.get_proc_addr(s))
    }
}

pub fn load() {
    let loader = Failover(
        DlProcLoader::current_module(),
        Failover(
            DlProcLoader::open(Path::new("libepoxy-0")),
            Failover(
                DlProcLoader::open(Path::new("libepoxy0")),
                DlProcLoader::open(Path::new("libepoxy")),
            ),
        ),
    );
    epoxy::load_with(fn_from(loader));
    GlView::load_with(epoxy::get_proc_addr)
}

fn main() {
    gtk::init().expect("Failed to initialize GTK.");

    let window = gtk::Window::new(gtk::WindowType::Toplevel);

    window.set_title("Hello, World");

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    window.set_default_size(640, 480);

    let gl_area = gtk::GLArea::new();
    gl_area.set_vexpand(true);
    gl_area.set_hexpand(true);
    gl_area.set_use_es(true);
    gl_area.set_required_version(2, 0);

    let gl_view: Rc<RefCell<GlView>> = Rc::new(RefCell::new(GlView::new()));

    let screen_buffer = Arc::new(ScreenBuffer::new(
        FilterType::NtscComposite,
        LogicalSize {
            width: 256,
            height: 240,
        },
    ));
    let running = true;
    let timer = Timer::new();
    let controller = StandardController::new();
    let keys = Buttons::empty();
    let paused = false;
    let frame_counter = 0;

    {
        let gl_view = gl_view.clone();
        let logical_size = screen_buffer.logical_size();
        gl_area.connect_realize(move |gl_area| {
            gl_area.make_current();
            load();
            gl_view.borrow_mut().on_load(logical_size);
        });
    }

    {
        let gl_view = gl_view.clone();
        gl_area.connect_resize(move |_, w, h| {
            gl_view.borrow_mut().on_resize(w as f32, h as f32);
        });
    }

    {
        let gl_view = gl_view.clone();
        let screen_buffer = screen_buffer.clone();
        gl_area.connect_render(move |_area, _context| {
            gl_view.borrow_mut().on_update(screen_buffer.as_ref());
            Inhibit(true)
        });
    }

    {
        let gl_view = gl_view.clone();
        window.connect_delete_event(move |win, _| {
            gl_view.borrow_mut().on_close();
            win.destroy();
            Inhibit(false)
        });
    }

    window.add(&gl_area);
    window.show_all();

    gtk::main();
}

struct Core<S: Sound + MixerInput> {
    core: Core,
    paused: bool,
    speaker: S,
    running: bool,
    timer: Timer,
    frame_counter: u64,
    controller: StandardController,
    screen_buffer: Arc<ScreenBuffer>,
}

impl<S: Sound + MixerInput> Core<S> {
    pub fn new(core: Core, speaker: S, screen_buffer: Arc<ScreenBuffer>) -> Self {
        Self {
            core,
            paused: false,
            speaker,
            running: true,
            timer: Timer::new(),
            frame_counter: 0,
            controller: StandardController::new(),
            screen_buffer,
        }
    }

    fn run(&mut self) {
        while self.running {
            if !self.paused {
                while !self.core.step(self.screen_buffer.as_mut(), &mut self.controller, &mut self.speaker) {}
                self.frame_counter += 1;
            }
            self.timer.wait();
        }
    }
}

