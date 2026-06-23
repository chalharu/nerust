use super::State;
use gtk::prelude::*;
use nerust_backend_opengl::GlRenderer;

use nerust_gui_shell::session::WindowSize;
use nerust_screen_video::{FrameBuffer, Renderer, SurfaceSize};
use shared_library::dynamic_library::DynamicLibrary;
use std::ptr;

#[derive(Debug, Default)]
pub(crate) struct GtkGlRenderer {
    view: Option<GlRenderer>,
}

impl GtkGlRenderer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn realize(&mut self, gl_area: &gtk::GLArea, state: &State) {
        gl_area.make_current();
        if let Some(error) = gl_area.error() {
            log::error!("{error}");
            return;
        }

        epoxy::load_with(|symbol| unsafe {
            match DynamicLibrary::open(None).unwrap().symbol(symbol) {
                Ok(value) => value,
                Err(error) => {
                    log::error!("{error}");
                    ptr::null()
                }
            }
        });
        GlRenderer::load_with(epoxy::get_proc_addr);

        let mut view = GlRenderer::new_shared();
        view.use_vao(true);
        view.on_load(state.render_profile()).unwrap();
        self.view = Some(view);
        self.reconfigure(gl_area.width() as u32, gl_area.height() as u32);
    }

    pub(crate) fn reconfigure(&mut self, width: u32, height: u32) {
        if let Some(view) = self.view.as_mut() {
            view.reconfigure(SurfaceSize::new(width, height));
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer) {
        if let Some(view) = self.view.as_mut() {
            view.render(frame_buffer);
        }
    }

    pub(crate) fn unrealize(&mut self) {
        if let Some(view) = self.view.as_mut() {
            view.on_close();
        }
        self.view = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Viewport {
    scale_x: f32,
    scale_y: f32,
    width: i32,
    height: i32,
}

fn viewport(
    window_size: WindowSize,
    width: i32,
    height: i32,
    scaling: Option<u32>,
    scale_factor: i32,
) -> Option<Viewport> {
    if width <= 0 || height <= 0 {
        return None;
    }

    let rate_x = f64::from(width) / f64::from(window_size.width);
    let rate_y = f64::from(height) / f64::from(window_size.height);
    let fit_rate = f64::min(rate_x, rate_y);
    let rate = scaling
        .map(|fixed| f64::min(f64::from(fixed), fit_rate))
        .unwrap_or(fit_rate);
    Some(Viewport {
        scale_x: (rate / rate_x) as f32,
        scale_y: (rate / rate_y) as f32,
        width: width.saturating_mul(scale_factor),
        height: height.saturating_mul(scale_factor),
    })
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::viewport;
    use nerust_gui_shell::session::WindowSize;

    #[test]
    fn viewport_preserves_aspect_ratio_for_letterboxed_width() {
        let viewport = viewport(
            WindowSize {
                width: 256.0,
                height: 240.0,
            },
            1024,
            600,
            None,
            1,
        )
        .unwrap();

        assert!(viewport.scale_x < 1.0);
        assert_eq!(viewport.scale_y, 1.0);
        assert_eq!(viewport.width, 1024);
        assert_eq!(viewport.height, 600);
    }

    #[test]
    fn viewport_caps_output_when_fixed_scaling_is_selected() {
        let viewport = viewport(
            WindowSize {
                width: 256.0,
                height: 240.0,
            },
            1280,
            960,
            Some(2),
            1,
        )
        .unwrap();

        assert!((viewport.scale_x - 0.4).abs() < f32::EPSILON);
        assert!((viewport.scale_y - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn viewport_rejects_non_positive_sizes() {
        assert!(
            viewport(
                WindowSize {
                    width: 256.0,
                    height: 240.0,
                },
                0,
                240,
                None,
                2,
            )
            .is_none()
        );
    }
}
