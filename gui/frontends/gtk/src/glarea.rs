use super::State;
use gtk::glib;
use gtk::prelude::*;
use nerust_backend_opengl::GlBackend;
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::settings::nes::scaling_factor;
use shared_library::dynamic_library::DynamicLibrary;
use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

pub(crate) struct GLAreaCore {
    gl_area: gtk::GLArea,
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
        ensure_view_loaded(&self.glarea(), self.state());
        self.resize(self.glarea().width(), self.glarea().height());
        self.glarea().queue_render();
    }

    fn resize(&self, width: i32, height: i32) {
        resize_view(&self.glarea(), self.state(), width, height);
    }

    fn render(&self) -> bool {
        render(&self.glarea(), self.state());
        true
    }

    fn unrealize(&self) {
        let state = self.state();
        let mut state = state.borrow_mut();
        if let Some(ref mut view) = state.view {
            view.on_close();
        }
        state.view = None;
    }

    fn tick(&self) -> bool {
        self.glarea().queue_render();
        true
    }
}

fn render(gl_area: &gtk::GLArea, state: Rc<RefCell<State>>) {
    gl_area.make_current();
    if let Some(e) = gl_area.error() {
        log::error!("{}", e);
        return;
    }
    let needs_resize = state.borrow().view.is_none();
    ensure_view_loaded(gl_area, state.clone());
    if needs_resize {
        resize_view(gl_area, state.clone(), gl_area.width(), gl_area.height());
    }
    if let Ok(state) = state.try_borrow()
        && let Some(ref view) = state.view
    {
        state.with_frame_buffer(|frame_buffer| view.on_update(frame_buffer));
    }
    unsafe {
        epoxy::Flush();
    }
}

fn ensure_view_loaded(gl_area: &gtk::GLArea, state: Rc<RefCell<State>>) {
    if state.borrow().view.is_some() {
        return;
    }
    let mut view = GlBackend::new();
    view.use_vao(true);
    gl_area.make_current();
    if let Some(e) = gl_area.error() {
        log::error!("{}", e);
        return;
    }
    epoxy::load_with(|s| unsafe {
        match DynamicLibrary::open(None).unwrap().symbol(s) {
            Ok(v) => v,
            Err(e) => {
                log::error!("{}", e);
                ptr::null()
            }
        }
    });
    GlBackend::load_with(epoxy::get_proc_addr);
    let mut state = state.borrow_mut();
    let video = state.video();
    view.on_load(
        video.presentation(),
        video
            .console_video_assets()
            .expect("NES session always has video assets"),
    )
    .unwrap();
    state.view = Some(view);
}

fn resize_view(gl_area: &gtk::GLArea, state: Rc<RefCell<State>>, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    gl_area.make_current();
    if let Some(error) = gl_area.error() {
        log::error!("{}", error);
        return;
    }
    let (window_size, scaling) = {
        let state = state.borrow();
        (
            state.window_size(),
            scaling_factor(state.settings_snapshot().local.video.scaling),
        )
    };
    let (scale_x, scale_y) = viewport_scale(window_size, width, height, scaling);
    let scale_factor = gl_area.scale_factor();
    let viewport_width = width.saturating_mul(scale_factor);
    let viewport_height = height.saturating_mul(scale_factor);

    if let Some(ref mut view) = state.borrow_mut().view {
        view.on_resize(scale_x, scale_y, viewport_width, viewport_height);
    }
}

fn viewport_scale(
    window_size: WindowSize,
    width: i32,
    height: i32,
    scaling: Option<u32>,
) -> (f32, f32) {
    let rate_x = f64::from(width) / f64::from(window_size.width);
    let rate_y = f64::from(height) / f64::from(window_size.height);
    let fit_rate = f64::min(rate_x, rate_y);
    let rate = scaling
        .map(|fixed| f64::min(f64::from(fixed), fit_rate))
        .unwrap_or(fit_rate);
    ((rate / rate_x) as f32, (rate / rate_y) as f32)
}

#[cfg(test)]
mod tests {
    use super::viewport_scale;
    use nerust_gui_session::core::WindowSize;

    #[test]
    fn viewport_scale_fits_to_window_by_default() {
        let (scale_x, scale_y) = viewport_scale(
            WindowSize {
                width: 256.0,
                height: 240.0,
            },
            512,
            480,
            None,
        );

        assert!((scale_x - 1.0).abs() < f32::EPSILON);
        assert!((scale_y - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn viewport_scale_caps_output_when_fixed_scaling_is_selected() {
        let (scale_x, scale_y) = viewport_scale(
            WindowSize {
                width: 256.0,
                height: 240.0,
            },
            1280,
            960,
            Some(2),
        );

        assert!((scale_x - 0.4).abs() < f32::EPSILON);
        assert!((scale_y - 0.5).abs() < f32::EPSILON);
    }
}
