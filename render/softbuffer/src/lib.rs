use std::num::NonZeroU32;

use log::{error, warn};
use nerust_render_traits::{
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
    reserved: nerust_render_traits::ReservedStrip,
    lut: LutEntry,
    resize_buffer: Vec<u32>,
    ntsc_buffer: Vec<u32>,
    osd: Option<nerust_render_traits::renderer::OsdLayer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeKernel {
    #[expect(unused)]
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
            reserved: nerust_render_traits::ReservedStrip::default(),
            osd: None,
            lut: LutEntry::new(),
            resize_buffer: Vec::new(),
            ntsc_buffer: Vec::new(),
        }
    }

    fn available_size(&self) -> SurfaceSize {
        let loss = self.reserved.height_loss(self.size.height);
        SurfaceSize::new(self.size.width, self.size.height.saturating_sub(loss))
    }

    fn resize_lut(&mut self) {
        self.lut.resize_lut(
            SurfaceSize {
                width: self.render_profile.logical_size.width as u32,
                height: self.render_profile.logical_size.height as u32,
            },
            self.render_profile.physical_size.width / self.render_profile.physical_size.height,
            self.available_size(),
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

    #[expect(clippy::too_many_arguments)]
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
                        // Callers supply [R, G, B, A]; repack to
                        // [B, G, R, A] for the vertical pass below.
                        let src_val = src_f(src_index);
                        // 重みを適用する
                        [
                            ((src_val[2] as u16 * w).saturating_add(0x80) >> 8) as u8, // Blue
                            ((src_val[1] as u16 * w).saturating_add(0x80) >> 8) as u8, // Green
                            ((src_val[0] as u16 * w).saturating_add(0x80) >> 8) as u8, // Red
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
                    .map(u32::from_ne_bytes); // NEを使うことで、速度を優先する

                resize_buffer[dst_index] = c.unwrap_or(0);
            }
        }

        // 縦方向に拡大。タップは最大2件のため行毎の確保はしない。
        for y in 0..dst_h {
            let lut_index = y * lut.lut_pixel_size();
            let mut taps = [(0u16, 0u16); 2];
            let mut tap_count = 0;
            for &tap in &lut.y_lut[lut_index..lut_index + lut.lut_pixel_size()] {
                if let Some(tap) = tap {
                    taps[tap_count] = tap;
                    tap_count += 1;
                }
            }
            let taps = &taps[..tap_count];
            for x in 0..dst_w {
                let dst_index = y * dst_w + x;
                let c = taps
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

    /// Tap coverage for one frame: every LUT entry the loops below
    /// touch must exist and point inside its target. A mismatch
    /// (stale profile, short rows) refuses the frame instead of
    /// panicking the UI thread. Valid frames pass unchanged: no
    /// pixel output moves.
    fn taps_cover(
        lut: &LutEntry,
        dst_w: usize,
        dst_h: usize,
        tap_stride: usize,
        resize_len: usize,
        ntsc_writes: usize,
        ntsc_len: usize,
    ) -> bool {
        let px = lut.lut_pixel_size();
        if lut.x_lut.len() < dst_w.saturating_mul(px) || lut.y_lut.len() < dst_h.saturating_mul(px)
        {
            return false;
        }
        if !lut.x_lut[..dst_w.saturating_mul(px)]
            .iter()
            .flatten()
            .all(|&(i, _)| (i as usize) < tap_stride)
        {
            return false;
        }
        if !lut.y_lut[..dst_h.saturating_mul(px)]
            .iter()
            .flatten()
            .all(|&(i, _)| (i as usize).saturating_add(1).saturating_mul(dst_w) <= resize_len)
        {
            return false;
        }
        ntsc_writes <= ntsc_len
    }

    fn palette_index(source_frame: &[u8], width: usize, x: i32, y: usize) -> u8 {
        if x < 0 || x as usize >= width {
            return BLACK_PALETTE_INDEX;
        }
        // Short rows render black instead of panicking the UI thread.
        source_frame
            .get(y.saturating_mul(width).saturating_add(x as usize))
            .copied()
            .unwrap_or(BLACK_PALETTE_INDEX)
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
        let offset = (row
            .saturating_mul(PALETTE_WIDTH)
            .saturating_add(color as usize))
        .saturating_mul(4);
        // Out-of-range palette data renders black instead of
        // panicking the UI thread; valid entries are untouched.
        buf.get(offset..offset.saturating_add(4))
            .and_then(|s| s.try_into().ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0)
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

impl SoftbufferRenderer {
    /// Straight-alpha blend of the OSD layer, clipped to the buffer.
    fn blend_osd(
        buffer: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        layer: &nerust_render_traits::renderer::OsdLayer,
    ) {
        let src_w = layer.width as usize;
        let src_h = layer.height as usize;
        if src_w == 0 || src_h == 0 || layer.rgba.len() < src_w * src_h * 4 {
            return;
        }
        for row in 0..src_h {
            let dst_y = layer.y as usize + row;
            if dst_y >= dst_h {
                break;
            }
            for col in 0..src_w {
                let dst_x = layer.x as usize + col;
                if dst_x >= dst_w {
                    break;
                }
                let s = (row * src_w + col) * 4;
                let alpha = f32::from(layer.rgba[s + 3]) / 255.0;
                if alpha == 0.0 {
                    continue;
                }
                // Buffer words are 0xAARRGGBB: big-endian bytes
                // are [A, R, G, B], not RGBA.
                let dst_pixel = buffer[dst_y * dst_w + dst_x].to_be_bytes();
                let blend = |src: u8, dst: u8| {
                    (f32::from(src) * alpha + f32::from(dst) * (1.0 - alpha)).round() as u8
                };
                buffer[dst_y * dst_w + dst_x] = u32::from_be_bytes([
                    255,
                    blend(layer.rgba[s], dst_pixel[1]),
                    blend(layer.rgba[s + 1], dst_pixel[2]),
                    blend(layer.rgba[s + 2], dst_pixel[3]),
                ]);
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

    fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
        self.resize_lut();
    }

    fn set_osd_layer(&mut self, layer: Option<nerust_render_traits::renderer::OsdLayer>) {
        self.osd = layer;
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
            // Profile/frame races (e.g. right after a system switch)
            // must skip, never render mismatched: every indexer below
            // trusts these dimensions, and a panic here would take
            // down the UI thread every frame until they agree again.
            error!(
                "Frame size mismatch: expected {}x{}, got {}x{}",
                self.render_profile.source_logical_size.width,
                self.render_profile.source_logical_size.height,
                frame.width(),
                frame.height()
            );
            return RenderResult::Skipped;
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
        // A failed surface resize (warned there, not fatal) leaves a
        // stale-sized buffer behind: refuse instead of overrunning it.
        if buffer.as_mut().len() < dst_w.saturating_mul(dst_h) {
            warn!(
                "softbuffer buffer too small: have {}, need {}x{}",
                buffer.as_mut().len(),
                dst_w,
                dst_h
            );
            return RenderResult::Skipped;
        }

        let top = (self.reserved.top as usize).min(dst_h);
        let avail_h = dst_h.saturating_sub((self.reserved.height_loss(self.size.height)) as usize);
        // Strip rows keep the cleared black: the surface buffer is
        // reused across frames, so stale game pixels must not linger.
        for row in buffer.as_mut()[..top.saturating_mul(dst_w)].iter_mut() {
            *row = 0;
        }
        for row in buffer.as_mut()[(top.saturating_add(avail_h)).saturating_mul(dst_w)..].iter_mut()
        {
            *row = 0;
        }

        let src_stride = frame.stride();
        let src = frame.as_ref();
        let game_rows = top.saturating_mul(dst_w);
        let game_len = avail_h.saturating_mul(dst_w);
        let dst = &mut buffer.as_mut()[game_rows..game_rows.saturating_add(game_len)];
        // let src_w = self.render_profile.source_logical_size.width;
        let src_h = self.render_profile.source_logical_size.height;
        // Short source images would panic every indexer below (the
        // profile match above only pins dimensions, not length).
        if src.len() < src_stride.saturating_mul(src_h) {
            warn!(
                "softbuffer source too small: have {}, need stride {}x{}",
                src.len(),
                src_stride,
                src_h
            );
            return RenderResult::Skipped;
        }

        // Tap coverage refuses stale-profile frames before the
        // indexers below (warned, not fatal: same pattern as above).
        let use_ntsc = matches!(frame.format(), PixelFormat::PaletteIndex { .. })
            && self.render_profile.ntsc_packed_rgba8.is_some();
        let tap_stride = if matches!(frame.format(), PixelFormat::Rgba) {
            src_stride / 4
        } else if use_ntsc {
            self.render_profile.logical_size.width
        } else {
            src_stride
        };
        let ntsc_writes = if use_ntsc {
            src_h.saturating_mul(self.render_profile.logical_size.width)
        } else {
            0
        };
        if !Self::taps_cover(
            &self.lut,
            dst_w,
            avail_h,
            tap_stride,
            self.resize_buffer.len(),
            ntsc_writes,
            self.ntsc_buffer.len(),
        ) {
            warn!("softbuffer tap coverage mismatch: refusing frame");
            return RenderResult::Skipped;
        }

        match frame.format() {
            PixelFormat::Rgba => {
                // Stride is bytes; the indexer below counts pixels
                // (the closure scales by 4), so convert once here.
                // PaletteIndex is 1bpp and needs no conversion.
                Self::rendering(
                    dst,
                    src_stride / 4,
                    src_h,
                    dst_w,
                    avail_h,
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
                        avail_h,
                        |i| self.ntsc_buffer[i].to_be_bytes(),
                        &self.lut,
                        &mut self.resize_buffer,
                    );
                } else {
                    // Out-of-range indices render black instead of
                    // panicking the UI thread (see mismatch guard).
                    Self::rendering(
                        dst,
                        src_stride,
                        src_h,
                        dst_w,
                        avail_h,
                        move |i| {
                            palette
                                .get(src.get(i).copied().unwrap_or(0) as usize)
                                .copied()
                                .unwrap_or(0)
                                .to_be_bytes()
                        },
                        &self.lut,
                        &mut self.resize_buffer,
                    );
                }
            }
        }

        if let Some(layer) = self.osd.as_ref() {
            Self::blend_osd(buffer.as_mut(), dst_w, dst_h, layer);
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

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_render_traits::{VideoFrameFormat, logical::LogicalSize, physical::PhysicalSize};

    #[test]
    fn blend_osd_writes_argb_words() {
        // Buffers are 0xAARRGGBB: opaque red over opaque black stays red.
        let mut buffer = vec![0xFF00_0000u32; 4];
        let layer = nerust_render_traits::renderer::OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![
                255, 0, 0, 255, // opaque red
                0, 0, 0, 0, // transparent keeps dst
            ],
        };
        SoftbufferRenderer::blend_osd(&mut buffer, 2, 2, &layer);
        assert_eq!(buffer[0], 0xFFFF_0000);
        assert_eq!(buffer[1], 0xFF00_0000);
    }

    fn test_profile() -> VideoRenderProfile {
        VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            physical_size: PhysicalSize {
                width: 256.0,
                height: 240.0,
            },
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        }
    }

    #[test]
    fn render_skips_mismatched_frame_instead_of_panicking() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        // GBA-sized frame against an NES profile: must skip, and
        // above all must not panic on the UI thread.
        let mut frame = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
        frame.resize(240, 160);
        assert_eq!(renderer.render(&frame), RenderResult::Skipped);
    }

    #[test]
    fn render_skips_without_surface() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert_eq!(renderer.render(&frame), RenderResult::Skipped);
    }

    #[test]
    fn rgba_render_survives_byte_stride() {
        // Regression: the Rgba indexer counts pixels while `stride()`
        // is bytes; a 256-byte-aligned stride once overran the source
        // and panicked the UI thread (GBA-sized Rgba frames).
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        let stride = 256usize;
        let mut src = vec![0u8; stride * 4];
        for y in 0..4 {
            for x in 0..4 {
                let o = y * stride + x * 4;
                src[o..o + 4].copy_from_slice(&[0x10, 0x20, 0x30, 0xFF]);
            }
        }
        let mut resize_buffer = vec![0u32; 8 * 4];
        let mut dst = vec![0u32; 8 * 8];
        SoftbufferRenderer::rendering(
            &mut dst,
            stride / 4,
            4,
            8,
            8,
            |i| src[i * 4..i * 4 + 4].try_into().unwrap(),
            &lut,
            &mut resize_buffer,
        );
        // Uniform input stays uniform in the interior: bilinear weights
        // sum to one (edges may go black where taps fall outside).
        let mut center = Vec::new();
        for y in 2..6 {
            for x in 2..6 {
                center.push(dst[y * 8 + x]);
            }
        }
        assert!(!center.is_empty() && center.iter().all(|&p| p == center[0] && p != 0));
    }

    #[test]
    fn rgba_render_preserves_channel_order() {
        // Regression: the horizontal pass once read [R, G, B, A] as
        // [A, B, G, R], turning blue into yellow and black into red.
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        for (input, expected) in [
            ([0x00u8, 0x00, 0xFF, 0xFF], 0xFF00_00FFu32),
            ([0x00u8, 0x00, 0x00, 0xFF], 0xFF00_0000u32),
            ([0xFFu8, 0x00, 0x00, 0xFF], 0xFFFF_0000u32),
        ] {
            let mut src = [0u8; 4 * 4 * 4];
            for px in src.as_chunks_mut::<4>().0 {
                *px = input;
            }
            let mut resize_buffer = vec![0u32; 8 * 4];
            let mut dst = vec![0u32; 8 * 8];
            SoftbufferRenderer::rendering(
                &mut dst,
                4,
                4,
                8,
                8,
                |i| src[i * 4..i * 4 + 4].try_into().unwrap(),
                &lut,
                &mut resize_buffer,
            );
            for y in 2..6 {
                for x in 2..6 {
                    assert_eq!(dst[y * 8 + x], expected, "input {input:02X?}");
                }
            }
        }
    }

    #[test]
    fn palette_index_out_of_range_renders_black() {
        let frame = vec![0x01u8; 4 * 4];
        assert_eq!(SoftbufferRenderer::palette_index(&frame, 4, 1, 1), 0x01);
        assert_eq!(
            SoftbufferRenderer::palette_index(&frame, 4, -1, 1),
            BLACK_PALETTE_INDEX
        );
        assert_eq!(
            SoftbufferRenderer::palette_index(&frame, 4, 4, 1),
            BLACK_PALETTE_INDEX
        );
        assert_eq!(
            SoftbufferRenderer::palette_index(&frame, 4, 1, 4),
            BLACK_PALETTE_INDEX
        );
    }

    #[test]
    fn read_entry_out_of_range_renders_black() {
        let buf = vec![0xABu8; 64 * 64 * 4];
        assert_eq!(SoftbufferRenderer::read_entry(&buf, 0, 0), 0xABAB_ABAB);
        assert_eq!(SoftbufferRenderer::read_entry(&buf, 63, 63), 0xABAB_ABAB);
        assert_eq!(SoftbufferRenderer::read_entry(&buf, 255, 63), 0);
        assert_eq!(SoftbufferRenderer::read_entry(&buf, 0, 10_000), 0);
    }

    #[test]
    fn taps_cover_accepts_well_formed_lut() {
        let mut lut = LutEntry::new();
        lut.resize_lut(
            SurfaceSize {
                width: 4,
                height: 4,
            },
            1.0,
            SurfaceSize {
                width: 8,
                height: 8,
            },
        );
        assert!(SoftbufferRenderer::taps_cover(&lut, 8, 8, 4, 8 * 4, 0, 0));
    }

    #[test]
    fn taps_cover_rejects_short_lut() {
        let lut = LutEntry::new();
        assert!(!SoftbufferRenderer::taps_cover(&lut, 8, 8, 4, 8 * 4, 0, 0));
    }
}
