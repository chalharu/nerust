//! Real-GL drive harness for unit tests. Needs an X display with
//! software GL (Xvfb + llvmpipe in CI); without `DISPLAY` the callers
//! skip gracefully so headless runs stay green.

use std::num::NonZeroU32;

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
    display: Display,
    config: Config,
}

pub(crate) struct TestContext {
    _surface: Surface<PbufferSurface>,
    pub(crate) context: PossiblyCurrentContext,
}

impl TestDisplay {
    /// Connect to the X display, or `None` when headless so local
    /// runs stay green.
    pub(crate) fn connect() -> Option<Self> {
        Self::connect_strict().or_else(|| {
            eprintln!("SKIP GL context tests: no GL display");
            None
        })
    }

    fn connect_strict() -> Option<Self> {
        std::env::var_os("DISPLAY")?;
        let display = unsafe {
            Display::new(
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0)),
                DisplayApiPreference::EglThenGlx(Box::new(|_reg| {})),
            )
            .ok()?
        };
        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let config = unsafe { display.find_configs(template).ok()? }.next()?;
        Some(Self { display, config })
    }

    pub(crate) fn into_parts(self) -> (Display, Config) {
        (self.display, self.config)
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
        crate::view::GlView::load_with(|name| {
            let proc_name = std::ffi::CString::new(name).expect("GL proc name has no interior nul");
            self.display.get_proc_address(&proc_name)
        });
        Some(TestContext {
            _surface: surface,
            context,
        })
    }
}
