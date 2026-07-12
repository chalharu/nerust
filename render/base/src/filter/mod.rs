mod filters;
pub mod presentation;
use crate::{LogicalSize, PhysicalSize, RGB};

pub const BLACK_PALETTE_INDEX: u8 = nerust_render_ntsc::BLACK;
pub const PALETTE_TEXTURE_WIDTH: u32 = 64;
pub const NTSC_TEXTURE_WIDTH: u32 = nerust_render_ntsc::SHADER_COLOR_COUNT as u32;
pub const NTSC_TEXTURE_HEIGHT: u32 =
    (nerust_render_ntsc::SHADER_PHASE_COUNT * nerust_render_ntsc::SHADER_PHASE_ENTRY_COUNT) as u32;

pub trait VideoFilter: Send {
    fn push(&mut self, value: u8, filter_func: &mut dyn FilterFunc);

    fn logical_size(&self) -> LogicalSize;
    fn physical_size(&self) -> PhysicalSize;
}

pub trait FilterFunc {
    fn filter_func(&mut self, value: RGB);
}

impl<F: filters::FilterUnit<Input = u8, Output = RGB>> VideoFilter for F {
    fn push(&mut self, value: u8, filter_func: &mut dyn FilterFunc) {
        filters::FilterUnit::push(self, value, &mut |x| filter_func.filter_func(x))
    }

    fn logical_size(&self) -> LogicalSize {
        filters::FilterUnit::logical_size(self)
    }

    fn physical_size(&self) -> PhysicalSize {
        filters::FilterUnit::physical_size(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FilterType {
    None,
    NtscRGB,
    NtscComposite,
    NtscSVideo,
}

impl FilterType {
    pub fn generate(self, size: LogicalSize) -> Box<dyn VideoFilter> {
        match self {
            FilterType::None => Box::new(filters::rgb::DirectRgb::new(size)),
            FilterType::NtscRGB => Box::new(filters::ntsc::NtscSimulator::rgb(size)),
            FilterType::NtscComposite => Box::new(filters::ntsc::NtscSimulator::composite(size)),
            FilterType::NtscSVideo => Box::new(filters::ntsc::NtscSimulator::svideo(size)),
        }
    }
}


