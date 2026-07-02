use std::num::NonZeroU32;

use log::{error, warn};
use nerust_render_base::{
    FrameBuffer, GpuFactory, GpuRenderer, OpaqueError, PixelFormat, RenderResult, RendererConfig,
    RendererError, SurfaceSize, VideoRenderProfile,
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

#[derive(Debug)]
pub struct SoftbufferRenderer {
    ctx: Option<Context<WindowHandlePair>>,
    surface: Option<Surface<WindowHandlePair, WindowHandlePair>>,
    render_profile: VideoRenderProfile,
    size: SurfaceSize,
    lut: LutEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeKernel {
    NearestNeighbor,
    Bilinear,
}

// LUT (Look-Up Table) entry
#[derive(Debug)]
struct LutEntry {
    x_lut: Vec<Vec<Option<usize>>>,
    y_lut: Vec<Vec<Option<usize>>>,
    kernel: ResizeKernel,
}

impl LutEntry {
    fn new() -> Self {
        Self {
            x_lut: Vec::new(),
            y_lut: Vec::new(),
            kernel: ResizeKernel::NearestNeighbor,
        }
    }

    fn inv_scale(
        source_size: SurfaceSize,
        source_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) -> (f32, f32) {
        let window_aspect = destination_size.width as f32 / destination_size.height as f32;
        let content_aspect = source_size.width as f32 / source_size.height as f32;

        let base_scale = if window_aspect > source_aspect_ratio {
            // Window is wider than content → letterbox (black bars on sides)
            source_size.height as f32 / destination_size.height as f32
        } else {
            // Window is taller than content → pillarbox (black bars on top/bottom)
            source_size.width as f32 / destination_size.width as f32
        };
        if content_aspect > source_aspect_ratio {
            (
                base_scale,
                base_scale * (content_aspect / source_aspect_ratio),
            )
        } else {
            (
                base_scale * (source_aspect_ratio / content_aspect),
                base_scale,
            )
        }
    }

    fn lut_reserve(lut: &mut Vec<Vec<Option<usize>>>, len: usize, inner_len: usize) {
        for (i, inner) in lut.iter_mut().enumerate() {
            if i < len {
                inner.reserve_exact(inner_len);
                inner.clear();
            }
        }
        while lut.len() < len {
            lut.push(Vec::with_capacity(inner_len));
        }
        if lut.len() > len {
            lut.truncate(len);
        }
    }

    fn resize_lut_nearest_neighbor(
        &mut self,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
        scale: (f32, f32),
    ) {
        Self::lut_reserve(&mut self.x_lut, 1, dst_w);
        Self::lut_reserve(&mut self.y_lut, 1, dst_h);

        for x in 0..dst_w {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_x = ((x as isize - (dst_w >> 1) as isize) as f32 * scale.0 + 0.5) as isize
                + (src_w >> 1) as isize;
            if src_x < 0 || src_x >= src_w as isize {
                self.x_lut[0].push(None);
            } else {
                self.x_lut[0].push(Some(src_x as usize));
            }
        }
        for y in 0..dst_h {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_y = ((y as isize - (dst_h >> 1) as isize) as f32 * scale.1 + 0.5) as isize
                + (src_h >> 1) as isize;
            if src_y < 0 || src_y >= src_h as isize {
                self.y_lut[0].push(None);
            } else {
                self.y_lut[0].push(Some(src_y as usize));
            }
        }
    }

    fn resize_lut(
        &mut self,
        source_size: SurfaceSize,
        source_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) {
        let dst_w = destination_size.width as usize;
        let dst_h = destination_size.height as usize;
        let src_w = source_size.width as usize;
        let src_h = source_size.height as usize;

        let scale = Self::inv_scale(source_size, source_aspect_ratio, destination_size);

        match self.kernel {
            ResizeKernel::NearestNeighbor => {
                self.resize_lut_nearest_neighbor(dst_w, dst_h, src_w, src_h, scale);
            }
            ResizeKernel::Bilinear => {
                // self.resize_lut_bilinear(dst_w, dst_h, src_w, src_h, scale);
            }
        }
    }
}

unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

impl SoftbufferRenderer {
    fn new(profile: &VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            render_profile: profile.clone(),
            size: SurfaceSize::new(0, 0),
            lut: LutEntry::new(),
        }
    }

    fn resize_lut(&mut self) {
        self.lut.resize_lut(
            SurfaceSize {
                width: self.render_profile.source_logical_size.width as u32,
                height: self.render_profile.source_logical_size.height as u32,
            },
            1.0, // TODO: NTSCフィルタ適用後はアスペクト比が変わるので、ここで計算する必要がある
            self.size.clone(),
        );
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
        self.resize_lut();
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.render_profile = profile.clone();
        self.resize_lut();
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        if frame.width() != self.render_profile.source_logical_size.width
            || frame.height() != self.render_profile.source_logical_size.height
        {
            error!(
                "Frame size mismatch: expected {}x{}, got {}x{}",
                self.render_profile.source_logical_size.width,
                self.render_profile.source_logical_size.height,
                frame.width(),
                frame.height()
            );
        }
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
                for (i, (x_lut, y_lut)) in
                    self.lut.x_lut.iter().zip(self.lut.y_lut.iter()).enumerate()
                {
                    for (y, &src_y) in y_lut.iter().enumerate() {
                        let dst_y_index = y * dst_w;
                        let Some(src_y) = src_y else {
                            if i == 0 {
                                for x in 0..dst_w {
                                    dst[dst_y_index + x] = 0;
                                }
                            }
                            continue;
                        };
                        let src_y_index = src_y * src_stride;
                        for (x, &src_x) in x_lut.iter().enumerate() {
                            let dst_index = dst_y_index + x;
                            let Some(src_x) = src_x else {
                                dst[dst_index] = 0;
                                continue;
                            };
                            let src_index = src_y_index + src_x * 4;

                            let c = u32::from_le_bytes([
                                src[src_index + 2], // Blue
                                src[src_index + 1], // Green
                                src[src_index],     // Red
                                src[src_index + 3], // Alpha
                            ]);
                            if i == 0 {
                                dst[dst_index] = c;
                            } else {
                                dst[dst_index] += c;
                            }
                        }
                    }
                }
            }
            PixelFormat::PaletteIndex { palette } => {
                // if self.ntsc_packed_rgba8.is_none() {
                for (i, (x_lut, y_lut)) in
                    self.lut.x_lut.iter().zip(self.lut.y_lut.iter()).enumerate()
                {
                    for (y, &src_y) in y_lut.iter().enumerate() {
                        let dst_y_index = y * dst_w;
                        let Some(src_y) = src_y else {
                            if i == 0 {
                                for x in 0..dst_w {
                                    dst[dst_y_index + x] = 0;
                                }
                            }
                            continue;
                        };
                        let src_y_index = src_y * src_stride;
                        for (x, &src_x) in x_lut.iter().enumerate() {
                            let dst_index = dst_y_index + x;
                            let Some(src_x) = src_x else {
                                dst[dst_index] = 0;
                                continue;
                            };
                            let src_index = src_y_index + src_x;
                            let c = palette[src[src_index] as usize];
                            // Convert from 0xRRGGBBAA to 0xAARRGGBB for softbuffer
                            if i == 0 {
                                dst[dst_index] = c.rotate_right(8);
                            } else {
                                dst[dst_index] += c.rotate_right(8);
                            }
                        }
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
