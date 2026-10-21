//! Real-GL drive harness for unit tests. Without `DISPLAY` the callers
//! skip gracefully so headless runs stay green.

use std::num::NonZeroU32;

use nerust_glwrap::bindings as gl;

use glutin::{
    config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder},
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _, PossiblyCurrentContext,
        Version,
    },
    display::{Display, DisplayApiPreference, GlDisplay as _},
    surface::{PbufferSurface, Surface, SurfaceAttributesBuilder},
};
use raw_window_handle::{RawDisplayHandle, XlibDisplayHandle};

pub(crate) struct TestDisplay {
    pub(crate) display: Display,
    config: Config,
}

pub(crate) struct TestContext {
    _dispatch: nerust_glwrap::bindings::DispatchGuard,
    _surface: Surface<PbufferSurface>,
    pub(crate) context: PossiblyCurrentContext,
}

/// Serializes context tests: two parallel pbuffer tests share one X
/// display, so drive them one at a time. Hold the guard for the test.
pub(crate) fn lock_display() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl TestDisplay {
    /// Connect to the X display, or `None` when headless so local
    /// runs stay green.
    pub(crate) fn connect() -> Option<Self> {
        std::env::var_os("DISPLAY")?;
        Some(Self::connect_strict().expect("DISPLAY is set but no GL display/config is available"))
    }

    fn connect_strict() -> Option<Self> {
        std::env::var_os("DISPLAY")?;
        let display = unsafe {
            Display::new(
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0)),
                DisplayApiPreference::EglThenGlx(Box::new(|_reg| {})),
            )
            .map_err(|error| eprintln!("GL test display initialization failed: {error:?}"))
            .ok()?
        };
        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let config = unsafe {
            display
                .find_configs(template)
                .map_err(|error| eprintln!("GL test config query failed: {error:?}"))
                .ok()?
        }
        .next()
        .or_else(|| {
            eprintln!("GL test display has no RGBA pbuffer config");
            None
        })?;
        Some(Self { display, config })
    }

    pub(crate) fn into_parts(self) -> (Display, Config) {
        (self.display, self.config)
    }

    /// Read back the current viewport box for assertions. Callers hold
    /// the context current (see `make_pbuffer_current`).
    pub(crate) fn viewport_of(out: &mut [i32; 4]) {
        unsafe { gl::GetIntegerv(gl::VIEWPORT, out.as_mut_ptr()) };
    }

    pub(crate) fn make_pbuffer_current(&self, width: u32, height: u32) -> Option<TestContext> {
        let attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(width.max(1))?,
            NonZeroU32::new(height.max(1))?,
        );
        let surface = unsafe {
            self.display
                .create_pbuffer_surface(&self.config, &attrs)
                .ok()?
        };
        let context = unsafe {
            self.display
                .create_context(
                    &self.config,
                    &ContextAttributesBuilder::new()
                        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                        .build(None),
                )
                .ok()?
        };
        let context = context.make_current(&surface).ok()?;
        let api = unsafe {
            nerust_glwrap::Context::load_with(|name| {
                self.display
                    .get_proc_address(&std::ffi::CString::new(name).unwrap())
            })
        };
        let dispatch = unsafe { api.enter() };
        Some(TestContext {
            _dispatch: dispatch,
            _surface: surface,
            context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TestDisplay;

    #[test]
    fn context_tables_are_thread_local_and_scopes_restore_dispatch() {
        use glutin::display::GlDisplay as _;
        use std::sync::{Arc, Barrier};
        let _serial = super::lock_display();
        let Some(display) = TestDisplay::connect() else {
            return;
        };
        drop(display);
        let barrier = Arc::new(Barrier::new(2));
        let workers: Vec<_> = [16, 24]
            .into_iter()
            .map(|width| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let display = TestDisplay::connect().expect("display became unavailable");
                    let _current = display
                        .make_pbuffer_current(32, 32)
                        .expect("pbuffer creation failed");
                    let api = unsafe {
                        nerust_glwrap::Context::load_with(|name| {
                            display
                                .display
                                .get_proc_address(&std::ffi::CString::new(name).unwrap())
                        })
                    };
                    let scope = unsafe { api.enter() };
                    assert!(nerust_glwrap::Current::assert_current(true).is_some());
                    for _ in 0..32 {
                        nerust_glwrap::raw::viewport(0, 0, width, width).unwrap();
                        let mut viewport = [0; 4];
                        unsafe {
                            api.with_current(|| {
                                nerust_glwrap::raw::get_viewport(&mut viewport).unwrap();
                            });
                        }
                        assert_eq!(viewport, [0, 0, width, width]);
                    }
                    // Deliberately omit one entry point for the same native
                    // context. The facade must panic before crossing FFI and
                    // restore the outer table when the nested scope unwinds.
                    let incomplete = unsafe {
                        nerust_glwrap::Context::load_with(|name| {
                            if name == "glViewport" {
                                std::ptr::null()
                            } else {
                                display
                                    .display
                                    .get_proc_address(&std::ffi::CString::new(name).unwrap())
                            }
                        })
                    };
                    assert!(
                        std::panic::catch_unwind(|| unsafe {
                            incomplete.with_current(|| nerust_glwrap::raw::viewport(0, 0, 1, 1))
                        })
                        .is_err()
                    );
                    let mut viewport = [0; 4];
                    nerust_glwrap::raw::get_viewport(&mut viewport).unwrap();
                    assert_eq!(viewport, [0, 0, width, width]);
                    drop(scope);
                    drop(_current);
                    assert!(nerust_glwrap::Current::assert_current(true).is_none());
                    assert!(std::panic::catch_unwind(nerust_glwrap::raw::check_frame).is_err());
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn display_lock_serializes_context_tests() {
        // Double-lock from one thread would deadlock; the guard
        // only proves the mutex is live and re-armable.
        drop(super::lock_display());
        drop(super::lock_display());
    }

    #[test]
    fn connect_skips_headless_without_display() {
        if std::env::var_os("DISPLAY").is_some() {
            eprintln!("SKIP: DISPLAY is set, connect may succeed");
            return;
        }
        assert!(TestDisplay::connect().is_none());
    }
}
