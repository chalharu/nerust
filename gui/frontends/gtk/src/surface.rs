use super::State;
use super::renderer::GtkRenderer;
use gtk::glib;
use gtk::prelude::*;
use nerust_screen_video::SurfaceSize;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkRenderer>>,
    state: Rc<RefCell<State>>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

pub(crate) trait SurfaceExtend {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface;
    fn tick(&self) -> bool;
}

impl SurfaceExtend for Surface {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface {
        state.borrow_mut().renderer_reload_pending = true;
        let renderer = Rc::new(RefCell::new(GtkRenderer::new()));
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.clone(),
            renderer,
            state,
        }));
        {
            let result = result.clone();
            let _ = window.add_tick_callback(move |_window, _frame_clock| {
                glib::ControlFlow::from(result.tick())
            });
        }
        result
    }

    fn tick(&self) -> bool {
        let s = self.borrow();
        if s.window.width() == 0 || s.window.height() == 0 {
            return true;
        }
        let mut needs_reinit = false;
        if let Ok(mut state) = s.state.try_borrow_mut() {
            needs_reinit = state.take_renderer_reload_pending();
            state.swap_frame_buffer();
        }
        if needs_reinit && let Ok(state) = s.state.try_borrow() {
            let size = SurfaceSize::new(s.window.width() as u32, s.window.height() as u32);
            log::info!("reinit size: {:?}", size);
            let profile = state.render_profile().clone();
            if let Some(surface) = s.window.surface()
                && let Some(display) = gdk::Display::default()
            {
                super::gdk_raw::with_raw_handles(&surface, &display, |wh, dh| {
                    s.renderer.borrow_mut().realize(wh, dh, size, &profile);
                });
            }
        }
        if let Ok(state) = s.state.try_borrow() {
            s.renderer.borrow_mut().render(state.frame_buffer());
        }
        true
    }
}
