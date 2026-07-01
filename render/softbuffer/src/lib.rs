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

struct RgbaCollector {
    buf: Vec<u32>,
}

impl FilterFunc for RgbaCollector {
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
    ntsc_active: bool,
    size: SurfaceSize,
}

unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

impl SoftbufferRenderer {
    fn new(profile: &VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            ntsc_active: profile.ntsc_packed_rgba8.is_some(),
            size: SurfaceSize::new(0, 0),
        }
    }

    fn render_rgba(
        dst: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        rgba: &[u32],
        src_w: usize,
        src_h: usize,
    ) {
        if src_w == 0 || src_h == 0 {
            return;
        }
        let scale = (dst_w as f64 / src_w as f64).min(dst_h as f64 / src_h as f64);
        let render_w = (src_w as f64 * scale).max(1.0) as usize;
        let render_h = (src_h as f64 * scale).max(1.0) as usize;
        let off_x = (dst_w - render_w) / 2;
        let off_y = (dst_h - render_h) / 2;

        for dy in 0..dst_h {
            let row = dy * dst_w;
            if dy < off_y || dy >= off_y + render_h {
                dst[row..row + dst_w].fill(0);
                continue;
            }
            let sy = ((dy - off_y) * src_h / render_h).min(src_h - 1);
            let src_row = sy * src_w;
            for dx in 0..dst_w {
                if dx < off_x || dx >= off_x + render_w {
                    dst[row + dx] = 0;
                    continue;
                }
                let sx = ((dx - off_x) * src_w / render_w).min(src_w - 1);
                dst[row + dx] = rgba.get(src_row + sx).copied().unwrap_or(0);
            }
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
        self.ntsc_active = profile.ntsc_packed_rgba8.is_some();
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
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

        let src_w = frame.width();
        let src_h = frame.height();
        let src_stride = frame.stride();
        let src = frame.as_ref();
        let dst = buffer.as_mut();

        match frame.format() {
            PixelFormat::Rgba => {
                let mut rgba = Vec::<u32>::with_capacity(src_w * src_h);
                for y in 0..src_h {
                    let base = y * src_stride;
                    for x in 0..src_w {
                        let si = base + x * 4;
                        rgba.push(u32::from_ne_bytes([
                            src[si + 2],
                            src[si + 1],
                            src[si],
                            src[si + 3],
                        ]));
                    }
                }
                Self::render_rgba(dst, dst_w, dst_h, &rgba, src_w, src_h);
            }
            PixelFormat::PaletteIndex { palette } => {
                if !self.ntsc_active {
                    let mut rgba = Vec::<u32>::with_capacity(src_w * src_h);
                    for y in 0..src_h {
                        let base = y * src_stride;
                        for x in 0..src_w {
                            let si = base + x;
                            let c = palette[src[si] as usize];
                            rgba.push(u32::from_ne_bytes([
                                (c >> 8) as u8,
                                (c >> 16) as u8,
                                (c >> 24) as u8,
                                c as u8,
                            ]));
                        }
                    }
                    Self::render_rgba(dst, dst_w, dst_h, &rgba, src_w, src_h);
                } else {
                    let source_size = LogicalSize {
                        width: src_w,
                        height: src_h,
                    };
                    let mut filter = FilterType::NtscComposite.generate(source_size);
                    let ntsc_phys = filter.physical_size();
                    let out_w = ntsc_phys.width as usize;
                    let out_h = ntsc_phys.height as usize;

                    let mut collector = RgbaCollector {
                        buf: Vec::with_capacity(out_w * out_h),
                    };
                    for y in 0..src_h {
                        let base = y * src_stride;
                        for x in 0..src_w {
                            filter.push(src[base + x], &mut collector);
                        }
                    }

                    Self::render_rgba(dst, dst_w, dst_h, &collector.buf, out_w, out_h);
                }
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
