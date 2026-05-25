use super::State;
use super::renderer::GtkGlRenderer;
use gtk::glib;
use gtk::prelude::*;
use nerust_gui_shell::settings::nes::scaling_factor;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct GLAreaCore {
    gl_area: gtk::GLArea,
    renderer: Rc<RefCell<GtkGlRenderer>>,
    state: Rc<RefCell<State>>,
}

pub(crate) type GLArea = Rc<RefCell<GLAreaCore>>;

pub(crate) trait GLAreaExtend {
    fn bind(gl_area: gtk::GLArea, state: Rc<RefCell<State>>) -> GLArea;
    fn realize(&self);
    fn resize(&self, width: i32, height: i32);
    fn render(&self) -> bool;
    fn unrealize(&self);
    fn tick(&self) -> bool;
    fn glarea(&self) -> gtk::GLArea;
    fn state(&self) -> Rc<RefCell<State>>;
}

impl GLAreaExtend for GLArea {
    fn glarea(&self) -> gtk::GLArea {
        self.borrow().gl_area.clone()
    }

    fn state(&self) -> Rc<RefCell<State>> {
        self.borrow().state.clone()
    }

    fn bind(gl_area: gtk::GLArea, state: Rc<RefCell<State>>) -> GLArea {
        gl_area.set_auto_render(false);

        let result = Rc::new(RefCell::new(GLAreaCore {
            gl_area: gl_area.clone(),
            renderer: Rc::new(RefCell::new(GtkGlRenderer::new())),
            state,
        }));
        {
            let result = result.clone();
            let _ = gl_area.connect_realize(move |_gl_area| result.realize());
        }
        {
            let result = result.clone();
            let _ = gl_area.connect_resize(move |_gl_area, w, h| {
                result.resize(w, h);
            });
        }
        {
            let result = result.clone();
            let _ = gl_area
                .connect_render(move |_gl_area, _context| glib::Propagation::from(result.render()));
        }
        {
            let result = result.clone();
            let _ = gl_area.connect_unrealize(move |_gl_area| result.unrealize());
        }
        {
            let result = result.clone();
            let _ = gl_area.add_tick_callback(move |_gl_area, _frame_clock| {
                glib::ControlFlow::from(result.tick())
            });
        }
        result
    }

    fn realize(&self) {
        let state = self.state();
        let state = state.borrow();
        self.borrow()
            .renderer
            .borrow_mut()
            .realize(&self.glarea(), &state);
        self.glarea().queue_render();
    }

    fn resize(&self, width: i32, height: i32) {
        let state = self.state();
        let state = state.borrow();
        self.borrow().renderer.borrow_mut().resize(
            &self.glarea(),
            state.window_size(),
            scaling_factor(state.settings_snapshot().local.video.window.scaling),
            width,
            height,
        );
    }

    fn render(&self) -> bool {
        render(&self.glarea(), self.borrow().renderer.clone(), self.state());
        true
    }

    fn unrealize(&self) {
        self.borrow().renderer.borrow_mut().unrealize();
    }

    fn tick(&self) -> bool {
        self.glarea().queue_render();
        true
    }
}

fn render(gl_area: &gtk::GLArea, renderer: Rc<RefCell<GtkGlRenderer>>, state: Rc<RefCell<State>>) {
    gl_area.make_current();
    if let Some(e) = gl_area.error() {
        log::error!("{}", e);
        return;
    }
    let needs_reload = { state.borrow_mut().take_renderer_reload_pending() };
    if needs_reload {
        renderer.borrow_mut().unrealize();
        if let Ok(state) = state.try_borrow() {
            renderer.borrow_mut().realize(gl_area, &state);
        }
    }
    if let Ok(state) = state.try_borrow() {
        if let Some(frame) = state.snapshot().video_frame {
            renderer.borrow().render(frame.bytes());
        }
    }
    unsafe {
        epoxy::Flush();
    }
}
