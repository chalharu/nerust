use std::num::NonZeroU32;

use log::{error, warn};
use nerust_render_base::{
    FrameBuffer, PixelFormat, SurfaceSize, VideoRenderProfile,
    filter::BLACK_PALETTE_INDEX,
    renderer::{GpuFactory, GpuRenderer, OpaqueError, RenderResult, RendererConfig, RendererError},
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
    resize_buffer: Vec<u32>,
    ntsc_buffer: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeKernel {
    #[allow(unused)]
    NearestNeighbor,
    Bilinear,
}

// LUT (Look-Up Table) entry
#[derive(Debug)]
struct LutEntry {
    x_lut: Vec<Option<(u16, u16)>>,
    y_lut: Vec<Option<(u16, u16)>>,
    kernel: ResizeKernel,
}

impl LutEntry {
    fn new() -> Self {
        Self {
            x_lut: Vec::new(),
            y_lut: Vec::new(),
            // kernel: ResizeKernel::NearestNeighbor,
            kernel: ResizeKernel::Bilinear,
        }
    }

    fn inv_scale(
        source_size: SurfaceSize,
        physical_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) -> (f32, f32) {
        let window_aspect = destination_size.width as f32 / destination_size.height as f32;
        let source_aspect = source_size.width as f32 / source_size.height as f32;

        if window_aspect > physical_aspect_ratio {
            // Window is wider than content → letterbox (black bars on sides)
            (
                source_size.height as f32 / destination_size.height as f32
                    * (source_aspect / physical_aspect_ratio),
                source_size.height as f32 / destination_size.height as f32,
            )
        } else {
            (
                source_size.width as f32 / destination_size.width as f32,
                source_size.width as f32 / destination_size.width as f32
                    * (physical_aspect_ratio / source_aspect),
            )
        }
    }

    fn lut_reserve(lut: &mut Vec<Option<(u16, u16)>>, len: usize) {
        lut.clear();
        lut.reserve_exact(len);
    }

    fn resize_lut_nearest_neighbor(
        &mut self,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
        scale: (f32, f32),
    ) {
        Self::lut_reserve(&mut self.x_lut, dst_w);
        Self::lut_reserve(&mut self.y_lut, dst_h);

        for x in 0..dst_w {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_x = ((x as isize - (dst_w >> 1) as isize) as f32 * scale.0 + 0.5) as isize
                + (src_w >> 1) as isize;
            if src_x < 0 || src_x >= src_w as isize {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x as u16, 256)));
            }
        }
        for y in 0..dst_h {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_y = ((y as isize - (dst_h >> 1) as isize) as f32 * scale.1 + 0.5) as isize
                + (src_h >> 1) as isize;
            if src_y < 0 || src_y >= src_h as isize {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y as u16, 256)));
            }
        }
    }

    fn resize_lut_bilinear(
        &mut self,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
        scale: (f32, f32),
    ) {
        Self::lut_reserve(&mut self.x_lut, dst_w * 2);
        Self::lut_reserve(&mut self.y_lut, dst_h * 2);

        for x in 0..dst_w {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_x =
                ((x as isize - (dst_w >> 1) as isize) as f32 * scale.0 + 0.5) + (src_w >> 1) as f32;
            let src_x_floor = src_x.floor() as isize;
            let src_x_ceil = src_x.ceil() as isize;
            let (mut weight_floor, mut weight_ceil) = if src_x_ceil == src_x_floor {
                (256, 0) // 同一のピクセルに対しては、floor側に全ての重みを割り当てる
            } else {
                (
                    ((src_x_ceil as f32 - src_x) * 256.0 + 0.5) as u16,
                    ((src_x - src_x_floor as f32) * 256.0 + 0.5) as u16,
                )
            };

            if src_x_floor < 0 || src_x_floor >= src_w as isize {
                weight_floor = 0;
                if weight_ceil != 0 {
                    weight_ceil = 256;
                }
            }
            if src_x_ceil < 0 || src_x_ceil >= src_w as isize {
                if weight_floor != 0 {
                    weight_floor = 256;
                }
                weight_ceil = 0;
            }

            if weight_floor == 0 {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x_floor as u16, weight_floor)));
            }
            if weight_ceil == 0 {
                self.x_lut.push(None);
            } else {
                self.x_lut.push(Some((src_x_ceil as u16, weight_ceil)));
            }
        }
        for y in 0..dst_h {
            // Center the source pixel coordinates based on the destination size and scaling factors
            let src_y =
                ((y as isize - (dst_h >> 1) as isize) as f32 * scale.1 + 0.5) + (src_h >> 1) as f32;
            let src_y_floor = src_y.floor() as isize;
            let src_y_ceil = src_y.ceil() as isize;
            let (mut weight_floor, mut weight_ceil) = if src_y_ceil == src_y_floor {
                (256, 0) // 同一のピクセルに対しては、floor側に全ての重みを割り当てる
            } else {
                (
                    ((src_y_ceil as f32 - src_y) * 256.0 + 0.5) as u16,
                    ((src_y - src_y_floor as f32) * 256.0 + 0.5) as u16,
                )
            };

            if src_y_floor < 0 || src_y_floor >= src_h as isize {
                weight_floor = 0;
                if weight_ceil != 0 {
                    weight_ceil = 256;
                }
            }
            if src_y_ceil < 0 || src_y_ceil >= src_h as isize {
                if weight_floor != 0 {
                    weight_floor = 256;
                }
                weight_ceil = 0;
            }

            if weight_floor == 0 {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y_floor as u16, weight_floor)));
            }
            if weight_ceil == 0 {
                self.y_lut.push(None);
            } else {
                self.y_lut.push(Some((src_y_ceil as u16, weight_ceil)));
            }
        }
    }

    fn resize_lut(
        &mut self,
        source_size: SurfaceSize,
        physical_aspect_ratio: f32,
        destination_size: SurfaceSize,
    ) {
        let dst_w = destination_size.width as usize;
        let dst_h = destination_size.height as usize;
        let src_w = source_size.width as usize;
        let src_h = source_size.height as usize;

        let scale = Self::inv_scale(source_size, physical_aspect_ratio, destination_size);

        match self.kernel {
            ResizeKernel::NearestNeighbor => {
                self.resize_lut_nearest_neighbor(dst_w, dst_h, src_w, src_h, scale);
            }
            ResizeKernel::Bilinear => {
                self.resize_lut_bilinear(dst_w, dst_h, src_w, src_h, scale);
            }
        }
    }

    fn lut_pixel_size(&self) -> usize {
        match self.kernel {
            ResizeKernel::NearestNeighbor => 1,
            ResizeKernel::Bilinear => 2,
        }
    }
}

unsafe impl Send for SoftbufferRenderer {}
unsafe impl Sync for SoftbufferRenderer {}

const NTSC_ROW_OFFSETS: [[usize; 6]; 7] = [
    [0, 19, 31, 7, 26, 38],
    [1, 20, 32, 8, 27, 39],
    [2, 14, 33, 9, 21, 40],
    [3, 15, 34, 10, 22, 41],
    [4, 16, 28, 11, 23, 35],
    [5, 17, 29, 12, 24, 36],
    [6, 18, 30, 13, 25, 37],
];
const NTSC_SOURCE_OFFSETS: [[i32; 6]; 7] = [
    [1, -1, 0, -2, -4, -3],
    [1, -1, 0, -2, -4, -3],
    [1, 2, 0, -2, -1, -3],
    [1, 2, 0, -2, -1, -3],
    [1, 2, 3, -2, -1, 0],
    [1, 2, 3, -2, -1, 0],
    [1, 2, 3, -2, -1, 0],
];

impl SoftbufferRenderer {
    fn new(profile: &VideoRenderProfile) -> Self {
        Self {
            ctx: None,
            surface: None,
            render_profile: profile.clone(),
            size: SurfaceSize::new(0, 0),
            lut: LutEntry::new(),
            resize_buffer: Vec::new(),
            ntsc_buffer: Vec::new(),
        }
    }

    fn resize_lut(&mut self) {
        self.lut.resize_lut(
            SurfaceSize {
                width: self.render_profile.logical_size.width as u32,
                height: self.render_profile.logical_size.height as u32,
            },
            self.render_profile.physical_size.width / self.render_profile.physical_size.height,
            self.size,
        );
        self.ntsc_buffer.resize(
            self.render_profile.logical_size.width * self.render_profile.logical_size.height,
            0,
        );
        self.resize_buffer.resize(
            self.size.width as usize * self.render_profile.logical_size.height,
            0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn rendering<F: Fn(usize) -> [u8; 4]>(
        dst: &mut [u32],
        src_stride: usize,
        src_h: usize,
        dst_w: usize,
        dst_h: usize,
        src_f: F,
        lut: &LutEntry,
        resize_buffer: &mut [u32],
    ) {
        // 横方向に拡大
        for y in 0..src_h {
            let dst_y_index = y * dst_w;
            let src_y_index = y * src_stride;
            for x in 0..dst_w {
                let dst_index = dst_y_index + x;
                let lut_index = x * lut.lut_pixel_size();
                let c = lut.x_lut[lut_index..lut_index + lut.lut_pixel_size()]
                    .iter()
                    .filter_map(|&x| x)
                    .map(|(i, w)| {
                        let src_index = src_y_index + i as usize;
                        let src_val = src_f(src_index);
                        // 重みを適用する
                        [
                            ((src_val[1] as u16 * w).saturating_add(0x80) >> 8) as u8, // Blue
                            ((src_val[2] as u16 * w).saturating_add(0x80) >> 8) as u8, // Green
                            ((src_val[3] as u16 * w).saturating_add(0x80) >> 8) as u8, // Red
                            ((src_val[0] as u16 * w).saturating_add(0x80) >> 8) as u8, // Alpha
                        ]
                    })
                    .reduce(|acc, val| {
                        // 4チャンネルの色を加算する
                        [
                            acc[0].saturating_add(val[0]), // Blue
                            acc[1].saturating_add(val[1]), // Green
                            acc[2].saturating_add(val[2]), // Red
                            acc[3].saturating_add(val[3]), // Alpha
                        ]
                    })
                    .map(u32::from_ne_bytes); // NEを使うことで、速度を優先する

                resize_buffer[dst_index] = c.unwrap_or(0);
            }
        }

        // 縦方向に拡大
        for y in 0..dst_h {
            let lut_index = y * lut.lut_pixel_size();
            let lut_values: Vec<_> = lut.y_lut[lut_index..lut_index + lut.lut_pixel_size()]
                .iter()
                .filter_map(|&x| x)
                .collect();
            for x in 0..dst_w {
                let dst_index = y * dst_w + x;
                let c = lut_values
                    .iter()
                    .copied()
                    .map(|(i, w)| {
                        let src_index = i as usize * dst_w + x;
                        let src_val = resize_buffer[src_index].to_ne_bytes(); // 格納時にNEを使ったので、ここでもNEを使う
                        // 重みを適用する, 横方向に拡大した結果を縦方向に拡大するので、横方向の色順序はそのまま使う
                        [
                            ((src_val[0] as u16 * w).saturating_add(0x80) >> 8) as u8, // Blue
                            ((src_val[1] as u16 * w).saturating_add(0x80) >> 8) as u8, // Green
                            ((src_val[2] as u16 * w).saturating_add(0x80) >> 8) as u8, // Red
                            ((src_val[3] as u16 * w).saturating_add(0x80) >> 8) as u8, // Alpha
                        ]
                    })
                    .reduce(|acc, val| {
                        // 4チャンネルの色を加算する
                        [
                            acc[0].saturating_add(val[0]), // Blue
                            acc[1].saturating_add(val[1]), // Green
                            acc[2].saturating_add(val[2]), // Red
                            acc[3].saturating_add(val[3]), // Alpha
                        ]
                    })
                    .map(u32::from_le_bytes);

                dst[dst_index] = c.unwrap_or(0);
            }
        }
    }

    fn palette_index(source_frame: &[u8], width: usize, x: i32, y: usize) -> u8 {
        if x < 0 || x >= width as i32 {
            return BLACK_PALETTE_INDEX;
        }
        source_frame[y * width + x as usize]
    }

    fn clamp_impl(io: u32) -> u32 {
        const NTSC_CLAMP_MASK: u32 = 0x300c03;
        const NTSC_CLAMP_ADD: u32 = 0x20280a02;

        let sub = (io >> 9) & NTSC_CLAMP_MASK;
        let clamp = NTSC_CLAMP_ADD.wrapping_sub(sub);
        (io | clamp) & clamp.wrapping_sub(sub)
    }

    fn rgb_out_impl(raw: u32) -> u32 {
        let rgb = ((raw >> 5) & 0x00ff0000) | ((raw >> 3) & 0x0000ff00) | ((raw >> 1) & 0x000000ff);
        (rgb << 8) | 0xff
    }

    fn read_entry(buf: &[u8], color: u8, row: usize) -> u32 {
        const PALETTE_WIDTH: usize = 64;
        let offset = (row * PALETTE_WIDTH + color as usize) * 4;
        u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap())
    }

    fn simulate_gpu_ntsc_rgba(
        src_h: usize,
        source_frame: &[u8],
        render_profile: &VideoRenderProfile,
        ntsc_buffer: &mut [u32],
    ) {
        let packed_entries = render_profile.ntsc_packed_rgba8.as_ref().unwrap();
        let src_w = render_profile.logical_size.width;

        for y in 0..src_h {
            let phase_row = (y % 3) * 42;
            for x in 0..src_w {
                let chunk = x / 7;
                let sample = x - chunk * 7;
                let base = (chunk * 3) as i32;
                let row_offsets = NTSC_ROW_OFFSETS[sample];
                let source_offsets = NTSC_SOURCE_OFFSETS[sample];
                let mut sum = 0_u32;

                for (source_offset, row_offset) in source_offsets.into_iter().zip(row_offsets) {
                    let color = Self::palette_index(
                        source_frame,
                        render_profile.source_logical_size.width,
                        base + source_offset,
                        y,
                    );
                    sum = sum.wrapping_add(Self::read_entry(
                        packed_entries,
                        color,
                        phase_row + row_offset,
                    ));
                }

                ntsc_buffer[y * src_w + x] = Self::rgb_out_impl(Self::clamp_impl(sum));
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
        // let src_w = self.render_profile.source_logical_size.width;
        let src_h = self.render_profile.source_logical_size.height;

        match frame.format() {
            PixelFormat::Rgba => {
                Self::rendering(
                    dst,
                    src_stride,
                    src_h,
                    dst_w,
                    dst_h,
                    move |i| src[i * 4..i * 4 + 4].try_into().unwrap(),
                    &self.lut,
                    &mut self.resize_buffer,
                );
            }
            PixelFormat::PaletteIndex { palette } => {
                if self.render_profile.ntsc_packed_rgba8.is_some() {
                    Self::simulate_gpu_ntsc_rgba(
                        src_h,
                        src,
                        &self.render_profile,
                        &mut self.ntsc_buffer,
                    );
                    Self::rendering(
                        dst,
                        self.render_profile.logical_size.width,
                        src_h,
                        dst_w,
                        dst_h,
                        |i| self.ntsc_buffer[i].to_ne_bytes(),
                        &self.lut,
                        &mut self.resize_buffer,
                    );
                } else {
                    Self::rendering(
                        dst,
                        src_stride,
                        src_h,
                        dst_w,
                        dst_h,
                        move |i| palette[src[i] as usize].to_le_bytes(),
                        &self.lut,
                        &mut self.resize_buffer,
                    );
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
