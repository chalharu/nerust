use super::State;
use gtk::prelude::*;
use nerust_backend_opengl::GlBackend;
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::settings::nes::scaling_factor;
use shared_library::dynamic_library::DynamicLibrary;
use std::ptr;

#[derive(Debug, Default)]
pub(crate) struct GtkGlRenderer {
    view: Option<GlBackend>,
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
        GlBackend::load_with(epoxy::get_proc_addr);

        let mut view = GlBackend::new();
        view.use_vao(true);
        let video = state.video();
        view.on_load(
            video.presentation(),
            video
                .console_video_assets()
                .expect("NES session always has video assets"),
        )
        .unwrap();
        self.view = Some(view);
        self.resize(
            gl_area,
            state.window_size(),
            scaling_factor(state.settings_snapshot().local.video.window.scaling),
            gl_area.width(),
            gl_area.height(),
        );
    }

    pub(crate) fn resize(
        &mut self,
        gl_area: &gtk::GLArea,
        window_size: WindowSize,
        scaling: Option<u32>,
        width: i32,
        height: i32,
    ) {
        let Some(viewport) = viewport(window_size, width, height, scaling, gl_area.scale_factor())
        else {
            return;
        };

        gl_area.make_current();
        if let Some(error) = gl_area.error() {
            log::error!("{error}");
            return;
        }

        if let Some(view) = self.view.as_mut() {
            view.on_resize(
                viewport.scale_x,
                viewport.scale_y,
                viewport.width,
                viewport.height,
            );
        }
    }

    pub(crate) fn render(&self, frame_buffer: &[u8]) {
        if let Some(view) = self.view.as_ref() {
            view.on_update(frame_buffer);
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
mod tests {
    use super::viewport;
    use nerust_gui_session::core::WindowSize;

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
