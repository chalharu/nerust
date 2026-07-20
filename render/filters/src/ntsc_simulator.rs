use nerust_render_traits::filter::FilterUnit;
use nerust_render_traits::logical::LogicalSize;
use nerust_render_traits::physical::PhysicalSize;
use nerust_render_traits::rgb::RGB;

#[derive(Debug)]
pub(crate) struct NtscSimulator {
    ntsc: nerust_render_ntsc::Engine,
    source: LogicalSize,
}

impl NtscSimulator {
    pub(crate) fn composite(source: LogicalSize) -> Self {
        Self {
            ntsc: nerust_render_ntsc::Engine::new(
                &nerust_render_ntsc::setup::Setup::Composite,
                source.width,
            ),
            source,
        }
    }

    pub(crate) fn svideo(source: LogicalSize) -> Self {
        Self {
            ntsc: nerust_render_ntsc::Engine::new(
                &nerust_render_ntsc::setup::Setup::SVideo,
                source.width,
            ),
            source,
        }
    }

    pub(crate) fn rgb(source: LogicalSize) -> Self {
        Self {
            ntsc: nerust_render_ntsc::Engine::new(
                &nerust_render_ntsc::setup::Setup::RGB,
                source.width,
            ),
            source,
        }
    }
}

impl FilterUnit for NtscSimulator {
    type Input = u8;
    type Output = RGB;

    fn push<F: FnMut(Self::Output)>(&mut self, value: Self::Input, next_func: &mut F) {
        self.ntsc.push(value, &mut |x| {
            next_func(RGB {
                red: x.red,
                green: x.green,
                blue: x.blue,
            })
        });
    }

    fn source_logical_size(&self) -> LogicalSize {
        self.source
    }

    fn source_physical_size(&self) -> PhysicalSize {
        PhysicalSize::from(self.source)
    }

    fn eval_logical_size(source: LogicalSize) -> LogicalSize {
        LogicalSize {
            width: nerust_render_ntsc::Engine::output_width(source.width),
            height: source.height,
        }
    }

    fn eval_physical_size(source: PhysicalSize) -> PhysicalSize {
        PhysicalSize {
            width: nerust_render_ntsc::Engine::output_width(source.width as usize) as f32,
            height: source.height * 2_f32,
        }
    }
}
