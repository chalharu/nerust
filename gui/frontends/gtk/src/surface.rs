use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};
use nerust_render_traits::{SurfaceSize, renderer::PipelineFactory};

use super::{State, renderer::GtkRenderer};

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkRenderer>>,
    pub(crate) state: Rc<RefCell<State>>,
    pub(crate) osd: RefCell<super::osd::GtkOsd>,
    last_size: Cell<SurfaceSize>,
    retry: RefCell<nerust_frontend_kernel::retry::RenderRetry>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

pub(crate) trait SurfaceExtend {
    fn bind(
        window: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn PipelineFactory>,
    ) -> Surface;
    fn tick(&self) -> bool;
}

impl SurfaceExtend for Surface {
    fn bind(
        window: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        factory: Rc<dyn PipelineFactory>,
    ) -> Surface {
        state.borrow_mut().renderer_reload_pending = true;
        let renderer = Rc::new(RefCell::new(GtkRenderer::new(factory)));
        let osd = super::osd::GtkOsd::new();
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.clone(),
            renderer,
            state,
            osd: RefCell::new(osd),
            last_size: Cell::new(SurfaceSize::new(0, 0)),
            retry: RefCell::new(Default::default()),
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
        // Clamp negatives: minimized/hidden windows report -1, and a
        // negative `as u32` cast would explode the surface size.
        let width = s.window.width().max(0) as u32;
        let height = s.window.height().max(0) as u32;
        if width == 0 || height == 0 {
            return true;
        }
        let scale = s.window.scale_factor().max(1) as u32;
        let physical_size =
            SurfaceSize::new(width.saturating_mul(scale), height.saturating_mul(scale));

        // Contended borrows skip loudly: the next tick retries (backoff),
        // never a silent dropped frame.
        let mut state = match s.state.try_borrow_mut() {
            Ok(state) => state,
            Err(_) => {
                log::warn!("gtk tick: state busy, skipping frame");
                return true;
            }
        };
        let now = std::time::Instant::now();
        state.swap_frame_buffer();
        let rebuild = state.renderer_reload_pending || !s.renderer.borrow().attached();
        if s.retry.borrow().ready(now)
            && (rebuild || physical_size != s.last_size.get())
            && let Some(profile) = state.render_profile().cloned()
        {
            let result = s.window.surface().and_then(|surf| {
                let display = gdk::Display::default()?;
                super::gdk_raw::with_raw_handles(&surf, &display, |wh, dh| {
                    let mut renderer = s.renderer.borrow_mut();
                    if rebuild {
                        renderer.realize(wh, dh, physical_size, &profile)
                    } else {
                        renderer.reattach(wh, dh, physical_size)
                    }
                })
            });
            match result {
                Some(Ok(())) => {
                    s.last_size.set(physical_size);
                    state.renderer_reload_pending = false;
                }
                failure => {
                    if s.retry.borrow_mut().failed(now) {
                        let detail = match failure {
                            Some(Err(error)) => error.to_string(),
                            _ => "native surface/display handles unavailable".into(),
                        };
                        state.session.report_external_error(
                            nerust_gui_shell::session::notification::FailureOrigin::Host,
                            format!("GTK renderer attach failed: {detail}"),
                        );
                    }
                }
            }
        }

        // Destination-generation invalidation and uploads precede the first
        // frame on a fresh surface, including a same-size recovery attach.
        let surface = self.clone();
        s.osd
            .borrow_mut()
            .poll(&surface, &mut state, &s.renderer, &s.window);

        if s.retry.borrow().ready(now)
            && let Some(fb) = state.frame_buffer()
        {
            match s.renderer.borrow_mut().render(fb, physical_size) {
                nerust_render_traits::renderer::RenderResult::Error => {
                    if s.retry.borrow_mut().failed(now) {
                        state.session.report_external_error(
                            nerust_gui_shell::session::notification::FailureOrigin::Host,
                            "GTK rendering failed; recreating renderer with backoff".into(),
                        );
                    }
                }
                nerust_render_traits::renderer::RenderResult::Presented => {
                    s.retry.borrow_mut().succeeded()
                }
                _ => {}
            }
        }

        true
    }
}
