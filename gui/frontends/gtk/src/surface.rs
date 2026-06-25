use super::State;
use super::renderer::GtkRenderer;
use gtk::glib;
use gtk::prelude::*;
use nerust_screen_video::SurfaceSize;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkRenderer>>,
    state: Rc<RefCell<State>>,
    last_physical_size: Cell<SurfaceSize>,
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
            last_physical_size: Cell::new(SurfaceSize::new(0, 0)),
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
            let app_size = SurfaceSize::new(s.window.width() as u32, s.window.height() as u32);
            let scale = s.window.scale_factor().max(1) as u32;
            let physical_size = SurfaceSize::new(
                (s.window.width() as u32).saturating_mul(scale),
                (s.window.height() as u32).saturating_mul(scale),
            );
            log::info!("reinit app={:?} physical={:?}", app_size, physical_size);
            let profile = state.render_profile().clone();
            if let Some(surface) = s.window.surface()
                && let Some(display) = gdk::Display::default()
            {
                super::gdk_raw::with_raw_handles(&surface, &display, |wh, dh| {
                    s.renderer.borrow_mut().realize(wh, dh, app_size, &profile);
                });
                // After realize, the surface (wgpu) was configured at app_size.
                // Reconfigure to physical_size so the swapchain matches the
                // actual pixel dimensions.  GlRenderer's reconfigure is a no-op.
                s.renderer.borrow_mut().reconfigure(physical_size);
                s.last_physical_size.set(physical_size);
            }
        }
        if let Ok(state) = s.state.try_borrow() {
            let scale = s.window.scale_factor().max(1) as u32;
            let physical_size = SurfaceSize::new(
                (s.window.width() as u32).saturating_mul(scale),
                (s.window.height() as u32).saturating_mul(scale),
            );
            if physical_size != s.last_physical_size.get() {
                s.last_physical_size.set(physical_size);
                // On resize, the native GDK surface may be recreated (Wayland
                // always does this; X11 + macOS may).  For wgpu this
                // invalidates the old wgpu surface, so we need to recreate it
                // from fresh handles.  OpenGL's glutin context is unaffected.
                if let Some(surf) = s.window.surface()
                    && let Some(disp) = gdk::Display::default()
                {
                    super::gdk_raw::with_raw_handles(&surf, &disp, |wh, dh| {
                        let _ = s
                            .renderer
                            .borrow_mut()
                            .recreate_surface(wh, dh, physical_size);
                    });
                }
            }
            s.renderer
                .borrow_mut()
                .render(state.frame_buffer(), physical_size);
        }
        true
    }
}
