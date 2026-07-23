use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};
use nerust_render_traits::{SurfaceSize, renderer::GpuFactory};

use super::{StateRef, renderer::GtkRenderer};

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkRenderer>>,
    state: StateRef,
    last_size: Cell<SurfaceSize>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

pub(crate) trait SurfaceExtend {
    fn bind(
        window: &gtk::ApplicationWindow,
        state: StateRef,
        factory: Rc<dyn GpuFactory>,
    ) -> Surface;
    fn tick(&self) -> bool;
}

impl SurfaceExtend for Surface {
    fn bind(
        window: &gtk::ApplicationWindow,
        state: StateRef,
        factory: Rc<dyn GpuFactory>,
    ) -> Surface {
        state.borrow_mut().renderer_reload_pending = true;
        let renderer = Rc::new(RefCell::new(GtkRenderer::new(factory)));
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.clone(),
            renderer,
            state,
            last_size: Cell::new(SurfaceSize::new(0, 0)),
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
        let width = s.window.width() as u32;
        let height = s.window.height() as u32;
        if width == 0 || height == 0 {
            return true;
        }
        let scale = s.window.scale_factor().max(1) as u32;
        let physical_size =
            SurfaceSize::new(width.saturating_mul(scale), height.saturating_mul(scale));

        if let Some(mut state) = s.state.try_borrow_mut() {
            // Recreate the wgpu surface on resize (GDK may recreate the native
            // surface, invalidating the old wgpu surface).  OpenGL is unaffected.
            if physical_size != s.last_size.get() {
                s.last_size.set(physical_size);
                if let Some(surf) = s.window.surface()
                    && let Some(display) = gdk::Display::default()
                {
                    super::gdk_raw::with_raw_handles(&surf, &display, |wh, dh| {
                        let _ = s.renderer.borrow_mut().reattach(wh, dh, physical_size);
                    });
                }
            }

            state.swap_frame_buffer();

            if state.take_renderer_reload_pending() {
                log::info!("reinit physical={:?}", physical_size);
                if let Some(surf) = s.window.surface()
                    && let Some(display) = gdk::Display::default()
                    && let Some(profile) = state.render_profile()
                {
                    super::gdk_raw::with_raw_handles(&surf, &display, |wh, dh| {
                        s.renderer
                            .borrow_mut()
                            .realize(wh, dh, physical_size, profile);
                    });
                }
            }

            if let Some(fb) = state.frame_buffer() {
                s.renderer.borrow_mut().render(fb, physical_size);
            }
        }

        true
    }
}
