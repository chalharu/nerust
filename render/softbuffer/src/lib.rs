use std::num::NonZeroU32;

use log::warn;
use nerust_render_base::{
    FrameBuffer, GpuFactory, GpuRenderer, LogicalSize, OpaqueError, PixelFormat, RGB, RenderResult,
    RendererConfig, RendererError, SurfaceSize, VideoRenderProfile,
    filter::{FilterFunc, FilterType},
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use softbuffer::{Context, Surface};

#[derive(Debug, Clone)]
struct WindowHandlePair {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl HasWindowHandle for WindowHandlePair {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        unsafe { Ok(WindowHandle::borrow_raw(self.window)) }
    }
}

impl HasDisplayHandle for WindowHandlePair {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        unsafe { Ok(DisplayHandle::borrow_raw(self.display)) }
    }
}

struct RgbaCollector<'a> {
    buf: &'a mut Vec<u32>,
}

impl FilterFunc for RgbaCollector<'_> {
    fn filter_func(&mut self, color: RGB) {
        self.buf.push(u32::from_ne_bytes([
            color.blue,
            color.green,
            color.red,
            u8::MAX,
        ]));
    }
}

#[derive(Debug)]
pub struct SoftbufferRenderer {
    ctx: Option<Context<WindowHandlePair>>,
    surface: Option<Surface<WindowHandlePair, WindowHandlePair>>,
    ntsc_packed_rgba8: Option<Box<[u8]>>,
    size: SurfaceSize,
    rgba: Vec<u32>,
}

unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

impl SoftbufferRenderer {
    fn new(profile: &VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            ntsc_packed_rgba8: profile.ntsc_packed_rgba8.clone(),
            size: SurfaceSize::new(0, 0),
            rgba: Vec::new(),
        }
    }

    fn inv_scale(&self, src_width: u32, src_height: u32) -> f32 {
        let window_aspect = self.size.width as f32 / self.size.height as f32;
        let content_aspect = src_width as f32 / src_height as f32;

        if window_aspect > content_aspect {
            // Window is wider than content → letterbox (black bars on sides)
            src_height as f32 / self.size.height as f32
        } else {
            // Window is taller than content → pillarbox (black bars on top/bottom)
            src_width as f32 / self.size.width as f32
        }
    }
}

impl GpuRenderer for SoftbufferRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        let pair = WindowHandlePair {
            window: window_handle,
            display: display_handle,
        };
        let ctx = Context::new(pair.clone()).map_err(|e| {
            RendererError::new("softbuffer context", Box::new(OpaqueError(e.to_string())))
        })?;
        let surface = Surface::new(&ctx, pair).map_err(|e| {
            RendererError::new("softbuffer surface", Box::new(OpaqueError(e.to_string())))
        })?;
        self.ctx = Some(ctx);
        self.surface = Some(surface);
        self.size = size;
        self.resize(size);
        Ok(())
    }

    fn detach(&mut self) {
        self.surface.take();
        self.ctx.take();
        self.size = SurfaceSize::new(0, 0);
    }

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        if let Some(surface) = self.surface.as_mut() {
            let w = NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN);
            let h = NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN);
            if let Err(e) = surface.resize(w, h) {
                warn!("softbuffer resize failed: {e}");
            }
        }
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.ntsc_packed_rgba8 = profile.ntsc_packed_rgba8.clone();
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let src_w = frame.width();
        let src_h = frame.height();
        let scale = self.inv_scale(src_w as u32, src_h as u32);

        let Some(surface) = self.surface.as_mut() else {
            return RenderResult::Skipped;
        };
        let dst_w = self.size.width as usize;
        let dst_h = self.size.height as usize;
        if dst_w == 0 || dst_h == 0 {
            return RenderResult::Skipped;
        }

        let mut buffer = match surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                warn!("softbuffer buffer_mut failed: {e}");
                return RenderResult::Error;
            }
        };

        let src_stride = frame.stride();
        let src = frame.as_ref();
        let dst = buffer.as_mut();

        match frame.format() {
            PixelFormat::Rgba => {
                for y in 0..dst_h {
                    for x in 0..dst_w {
                        let dst_index = y * dst_w + x;

                        // Center the source pixel coordinates based on the destination size and scaling factors
                        let src_x = ((x as isize - (dst_w >> 1) as isize) as f32 * scale + 0.5)
                            as isize
                            + (src_w >> 1) as isize;
                        let src_y = ((y as isize - (dst_h >> 1) as isize) as f32 * scale + 0.5)
                            as isize
                            + (src_h >> 1) as isize;
                        if src_x < 0
                            || src_x >= src_w as isize
                            || src_y < 0
                            || src_y >= src_h as isize
                        {
                            dst[dst_index] = 0; // Fill with black if out of bounds
                            continue;
                        }
                        let src_index = src_y as usize * src_stride + src_x as usize * 4;

                        dst[dst_index] = u32::from_le_bytes([
                            src[src_index + 2], // Blue
                            src[src_index + 1], // Green
                            src[src_index],     // Red
                            src[src_index + 3], // Alpha
                        ]);
                    }
                }
            }
            PixelFormat::PaletteIndex { palette } => {
                // if self.ntsc_packed_rgba8.is_none() {
                for y in 0..dst_h {
                    for x in 0..dst_w {
                        let dst_index = y * dst_w + x;

                        // Center the source pixel coordinates based on the destination size and scaling factors
                        let src_x = ((x as isize - (dst_w >> 1) as isize) as f32 * scale + 0.5)
                            as isize
                            + (src_w >> 1) as isize;
                        let src_y = ((y as isize - (dst_h >> 1) as isize) as f32 * scale + 0.5)
                            as isize
                            + (src_h >> 1) as isize;
                        if src_x < 0
                            || src_x >= src_w as isize
                            || src_y < 0
                            || src_y >= src_h as isize
                        {
                            dst[dst_index] = 0; // Fill with black if out of bounds
                            continue;
                        }
                        let src_index = src_y as usize * src_stride + src_x as usize;
                        let c = palette[src[src_index] as usize];

                        // Convert from 0xRRGGBBAA to 0xAARRGGBB for softbuffer
                        dst[dst_index] = c.rotate_right(8);
                    }
                }
                // } else {
                // let source_size = LogicalSize {
                //     width: src_w,
                //     height: src_h,
                // };
                // let mut filter = FilterType::NtscComposite.generate(source_size);
                // filter.push(value, filter_func);
                // }
            }
        }

        match buffer.present() {
            Ok(()) => RenderResult::Presented,
            Err(e) => {
                warn!("softbuffer present failed: {e}");
                RenderResult::Error
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SoftbufferFactory;

impl GpuFactory for SoftbufferFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        _display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        Ok(Box::new(SoftbufferRenderer::new(&config.render_profile)))
    }
}
