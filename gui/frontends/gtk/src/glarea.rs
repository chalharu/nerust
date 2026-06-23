use super::State;
use super::renderer::GtkGlRenderer;
use gtk::glib;
use gtk::prelude::*;
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
        self.borrow()
            .renderer
            .borrow_mut()
            .reconfigure(width as u32, height as u32);
    }

    fn render(&self) -> bool {
        let gl_area = self.glarea();
        gl_area.make_current();
        if let Some(e) = gl_area.error() {
            log::error!("{}", e);
            return true;
        }
        let needs_reload = { self.state().borrow_mut().take_renderer_reload_pending() };
        if needs_reload {
            self.borrow().renderer.borrow_mut().unrealize();
            if let Ok(state) = self.state().try_borrow() {
                self.borrow()
                    .renderer
                    .borrow_mut()
                    .realize(&gl_area, &state);
            }
        }
        if let Ok(mut state) = self.state().try_borrow_mut() {
            state.swap_frame_buffer();
        }
        if let Ok(state) = self.state().try_borrow() {
            self.borrow()
                .renderer
                .borrow_mut()
                .render(state.frame_buffer());
        }
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
