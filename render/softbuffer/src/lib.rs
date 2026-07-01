use std::num::NonZeroU32;

use log::warn;
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
}

// SAFETY: SoftbufferRenderer only accesses the native window through softbuffer
// API calls that are thread-safe. The raw window handles are never dereferenced
// directly; they are only passed to softbuffer which handles platform safety.
unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

impl SoftbufferRenderer {
    fn new(render_profile: VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            render_profile,
            size: SurfaceSize::new(0, 0),
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
        self.render_profile = profile.clone();
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
                for dy in 0..dst_h {
                    let sy = dy * src_h / dst_h;
                    for dx in 0..dst_w {
                        let sx = dx * src_w / dst_w;
                        let si = sy * src_stride + sx * 4;
                        dst[dy * dst_w + dx] =
                            u32::from_ne_bytes([src[si], src[si + 1], src[si + 2], src[si + 3]]);
                    }
                }
            }
            PixelFormat::PaletteIndex { palette } => {
                for dy in 0..dst_h {
                    let sy = dy * src_h / dst_h;
                    for dx in 0..dst_w {
                        let sx = dx * src_w / dst_w;
                        let si = sy * src_stride + sx;
                        let c = palette[src[si] as usize];
                        dst[dy * dst_w + dx] = u32::from_ne_bytes([
                            (c >> 24) as u8,
                            (c >> 16) as u8,
                            (c >> 8) as u8,
                            c as u8,
                        ]);
                    }
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
        Ok(Box::new(SoftbufferRenderer::new(
            config.render_profile.clone(),
        )))
    }
}
