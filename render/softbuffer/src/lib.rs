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
    /// Last tap-coverage verdict with its inputs; skips the scan on a key match.
    taps_cache: Option<(TapKey, bool)>,
    /// Frame counter for the periodic full-rescan revalidation.
    scan_counter: u64,
}

/// Tap-coverage inputs for one frame shape. Caches negative verdicts
/// too: the key carries buffer lengths plus `lut_rev`, so transient
/// LUT/buffer growth during `resize_lut` changes the key and the stale
/// `false` never survives the resize that fixes it. `src` geometry,
/// byte stride, `avail_h`, kernel, and reserved strips are part of the
/// key so a stale `true` cannot survive a profile/strip change.
/// `dst_h` is deliberately *not* stored: it always equals
/// `avail_h + height_loss` (asserted at the call site), so storing it
/// would only admit inconsistent keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TapKey {
    dst_w: usize,
    avail_h: usize,
    src_w: usize,
    src_h: usize,
    /// Byte stride of the source frame. Distinct from `tap_stride`
    /// (which is pixels or indices): two strides can share a
    /// `tap_stride` under integer division yet feed the indexer
    /// different rows (e.g. byte strides 256 vs 258 at bpp 4).
    src_stride: usize,
    tap_stride: usize,
    resize_len: usize,
    ntsc_writes: usize,
    ntsc_len: usize,
    lut_rev: u64,
    kernel_bilinear: bool,
    reserved_top: u32,
    reserved_bottom: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeKernel {
    // No selection knob yet: Bilinear is hardcoded in `LutEntry::new`.
    // Kept (not deleted) so the nearest path stays compilable beside
    // the bilinear one; arms dispatch on `self.kernel` as usual.
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
    rev: u64,
}

impl LutEntry {
    fn new() -> Self {
        Self {
            x_lut: Vec::new(),
            y_lut: Vec::new(),
            // kernel: ResizeKernel::NearestNeighbor,
            kernel: ResizeKernel::Bilinear,
            rev: 0,
        }
    }

    /// Generation counter: any LUT rebuild invalidates tap verdicts.
    fn bump_rev(&mut self) {
        self.rev = self.rev.wrapping_add(1);
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
        self.bump_rev();
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
        self.bump_rev();
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
        self.bump_rev();
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
            taps_cache: None,
            scan_counter: 0,
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
        // Hoisted tap width: no per-pixel method call on the hot path.
        let px = lut.lut_pixel_size();
        Self::horizontal_pass(src_stride, src_h, dst_w, &src_f, lut, px, resize_buffer);
        Self::vertical_pass(dst, dst_w, dst_h, lut, px, resize_buffer);
    }

    /// Horizontal two-tap loop with consistent LE packing. Canonical
    /// order: the `src_f` indexer returns `[R, G, B, A]` bytes and the
    /// buffer word is `0xAARRGGBB` little-endian (byte 0 = blue).
    /// Weights sum to one in the interior; edges go black where taps
    /// fall outside.
    fn horizontal_pass<F: Fn(usize) -> [u8; 4]>(
        src_stride: usize,
        src_h: usize,
        dst_w: usize,
        src_f: &F,
        lut: &LutEntry,
        px: usize,
        resize_buffer: &mut [u32],
    ) {
        for y in 0..src_h {
            let dst_y_index = y * dst_w;
            let src_y_index = y * src_stride;
            for x in 0..dst_w {
                let dst_index = dst_y_index + x;
                let base = x * px;
                // Covered by `taps_cover`; a short LUT here would
                // silently black out the row via the `unwrap_or` below.
                debug_assert!(lut.x_lut.len() >= base + px);
                let mut acc = [0u16; 4];
                let mut touched = false;
                for tap in lut.x_lut.get(base..base + px).unwrap_or(&[]) {
                    let Some((i, w)) = tap else { continue };
                    let src_index = src_y_index + *i as usize;
                    let src_val = src_f(src_index);
                    let w = *w;
                    acc[0] = acc[0].saturating_add((src_val[2] as u16 * w + 0x80) >> 8);
                    acc[1] = acc[1].saturating_add((src_val[1] as u16 * w + 0x80) >> 8);
                    acc[2] = acc[2].saturating_add((src_val[0] as u16 * w + 0x80) >> 8);
                    acc[3] = acc[3].saturating_add((src_val[3] as u16 * w + 0x80) >> 8);
                    touched = true;
                }
                let word = if touched {
                    u32::from_le_bytes([
                        acc[0].min(255) as u8,
                        acc[1].min(255) as u8,
                        acc[2].min(255) as u8,
                        acc[3].min(255) as u8,
                    ])
                } else {
                    0
                };
                if dst_index < resize_buffer.len() {
                    resize_buffer[dst_index] = word;
                }
            }
        }
    }

    /// Vertical pass over the horizontal result. Taps are at most 2.
    /// Reads buffer words (`0xAARRGGBB` LE) back with `to_le_bytes`,
    /// the exact inverse of the horizontal pack above.
    fn vertical_pass(
        dst: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        lut: &LutEntry,
        px: usize,
        resize_buffer: &[u32],
    ) {
        for y in 0..dst_h {
            let base = y * px;
            // Covered by `taps_cover`; a short LUT would silently
            // black out the row via the `get` below.
            debug_assert!(lut.y_lut.len() >= base + px);
            let mut taps = [(0u16, 0u16); 2];
            let mut tap_count = 0;
            if let Some(slice) = lut.y_lut.get(base..base + px) {
                for tap in slice.iter().flatten() {
                    if tap_count < 2 {
                        taps[tap_count] = *tap;
                        tap_count += 1;
                    }
                }
            }
            let taps = &taps[..tap_count];
            for x in 0..dst_w {
                let dst_index = y * dst_w + x;
                if dst_index >= dst.len() {
                    break;
                }
                let mut acc = [0u16; 4];
                let mut touched = false;
                for (i, w) in taps.iter().copied() {
                    let src_index = i as usize * dst_w + x;
                    let Some(word) = resize_buffer.get(src_index) else {
                        continue;
                    };
                    let src_val = word.to_le_bytes();
                    acc[0] = acc[0].saturating_add((src_val[0] as u16 * w + 0x80) >> 8);
                    acc[1] = acc[1].saturating_add((src_val[1] as u16 * w + 0x80) >> 8);
                    acc[2] = acc[2].saturating_add((src_val[2] as u16 * w + 0x80) >> 8);
                    acc[3] = acc[3].saturating_add((src_val[3] as u16 * w + 0x80) >> 8);
                    touched = true;
                }
                dst[dst_index] = if touched {
                    u32::from_le_bytes([
                        acc[0].min(255) as u8,
                        acc[1].min(255) as u8,
                        acc[2].min(255) as u8,
                        acc[3].min(255) as u8,
                    ])
                } else {
                    0
                };
            }
        }
    }

    /// Tap coverage for one frame: every touched LUT entry must exist and
    /// point inside its target; mismatch refuses the frame instead of panicking.
    /// Pure in its inputs so LUT fixtures test it without a renderer.
    fn taps_cover(
        lut: &LutEntry,
        resize_len: usize,
        ntsc_len: usize,
        dst_w: usize,
        dst_h: usize,
        tap_stride: usize,
        ntsc_writes: usize,
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

    /// Cached [`Self::taps_cover`]: geometry rarely changes, so a key
    /// match skips the per-frame LUT scan. The key carries `src`
    /// geometry, `avail_h`, kernel, and strips, so a stale `true`
    /// cannot survive a shape change. Every 256th hit forces a full
    /// rescan (`debug_assert` verifies the cache); release keeps the
    /// cached verdict for speed.
    fn taps_covered(
        &mut self,
        dst_w: usize,
        avail_h: usize,
        src_stride: usize,
        tap_stride: usize,
        ntsc_writes: usize,
    ) -> bool {
        let key = TapKey {
            dst_w,
            avail_h,
            src_w: self.render_profile.source_logical_size.width,
            src_h: self.render_profile.source_logical_size.height,
            src_stride,
            tap_stride,
            resize_len: self.resize_buffer.len(),
            ntsc_writes,
            ntsc_len: self.ntsc_buffer.len(),
            lut_rev: self.lut.rev,
            kernel_bilinear: matches!(self.lut.kernel, ResizeKernel::Bilinear),
            reserved_top: self.reserved.top,
            reserved_bottom: self.reserved.bottom,
        };
        if let Some((cached_key, covered)) = &self.taps_cache
            && *cached_key == key
        {
            self.scan_counter = self.scan_counter.wrapping_add(1);
            if self.scan_counter.is_multiple_of(256) {
                let fresh = Self::taps_cover(
                    &self.lut,
                    self.resize_buffer.len(),
                    self.ntsc_buffer.len(),
                    dst_w,
                    avail_h,
                    tap_stride,
                    ntsc_writes,
                );
                debug_assert_eq!(
                    fresh, *covered,
                    "tap cache stale-true: periodic rescan disagrees"
                );
                return fresh;
            }
            return *covered;
        }
        let covered = Self::taps_cover(
            &self.lut,
            self.resize_buffer.len(),
            self.ntsc_buffer.len(),
            dst_w,
            avail_h,
            tap_stride,
            ntsc_writes,
        );
        self.scan_counter = self.scan_counter.wrapping_add(1);
        self.taps_cache = Some((key, covered));
        covered
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

/// Validated frame geometry shared by the blit path. Computed once:
/// dimensions, tap coverage, and strides feed both the guards and
/// the indexer setup without a second scan.
struct ValidatedFrame {
    dst_w: usize,
    dst_h: usize,
    avail_h: usize,
    src_stride: usize,
    src_h: usize,
    tap_stride: usize,
    ntsc_writes: usize,
}

impl SoftbufferRenderer {
    /// Validate dimensions without touching the surface or the tap
    /// cache. Pure read: tap coverage (which memoizes) runs separately
    /// at the call site, so this half stays `&self`. Also
    /// pre-validates `stride >= width * bpp` so the fallible indexers
    /// below never read out of bounds on the UI thread.
    fn validate_frame(&self, frame: &FrameBuffer) -> Option<ValidatedFrame> {
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
            return None;
        }
        let dst_w = self.size.width as usize;
        let dst_h = self.size.height as usize;
        if dst_w == 0 || dst_h == 0 || self.surface.is_none() {
            return None;
        }
        let avail_h = dst_h.saturating_sub(self.reserved.height_loss(self.size.height) as usize);
        let src_stride = frame.stride();
        let src_h = self.render_profile.source_logical_size.height;
        let src_w = self.render_profile.source_logical_size.width;
        let bpp = frame.format().bytes_per_pixel();
        // Stride must cover one tight row; otherwise the pixel indexer
        // would read past the source on the UI thread.
        if src_stride < src_w.saturating_mul(bpp) {
            warn!(
                "softbuffer stride too small: have {}, need {}",
                src_stride,
                src_w.saturating_mul(bpp)
            );
            return None;
        }
        let use_ntsc = matches!(frame.format(), PixelFormat::PaletteIndex { .. })
            && self.render_profile.ntsc_packed_rgba8.is_some();
        let tap_stride = if matches!(frame.format(), PixelFormat::Rgba) {
            src_stride / 4
        } else if use_ntsc {
            self.render_profile.logical_size.width
        } else {
            src_stride
        };
        if tap_stride == 0 {
            return None;
        }
        let ntsc_writes = if use_ntsc {
            src_h.saturating_mul(self.render_profile.logical_size.width)
        } else {
            0
        };
        Some(ValidatedFrame {
            dst_w,
            dst_h,
            avail_h,
            src_stride,
            src_h,
            tap_stride,
            ntsc_writes,
        })
    }

    /// Straight-alpha blend of the OSD layer, clipped to the buffer.
    fn blend_osd(
        buffer: &mut [u32],
        dst_w: usize,
        dst_h: usize,
        layer: &nerust_render_traits::renderer::OsdLayer,
    ) {
        let src_w = layer.width as usize;
        let src_h = layer.height as usize;
        let need = src_w.checked_mul(src_h).and_then(|px| px.checked_mul(4));
        let Some(need) = need else { return };
        if src_w == 0 || src_h == 0 || layer.rgba.len() < need {
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
                let alpha = u32::from(layer.rgba[s + 3]);
                if alpha == 0 {
                    continue;
                }
                // Buffer words are 0xAARRGGBB. Shift out channels
                // directly instead of `to_be_bytes` round-trips.
                let dst_word = buffer[dst_y * dst_w + dst_x];
                let (dr, dg, db) = (
                    ((dst_word >> 16) & 0xff) as u8,
                    ((dst_word >> 8) & 0xff) as u8,
                    (dst_word & 0xff) as u8,
                );
                let blend = |src: u8, dst: u8| {
                    ((u32::from(src) * alpha + u32::from(dst) * (255 - alpha) + 127) / 255) as u8
                };
                buffer[dst_y * dst_w + dst_x] = 0xff00_0000
                    | (u32::from(blend(layer.rgba[s], dr)) << 16)
                    | (u32::from(blend(layer.rgba[s + 1], dg)) << 8)
                    | u32::from(blend(layer.rgba[s + 2], db));
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
        use nerust_render_traits::renderer::OsdLayer;
        if OsdLayer::same_option(self.osd.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd = layer;
    }

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        self.render_profile = profile.clone();
        self.resize_lut();
        Ok(())
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let Some(valid) = self.validate_frame(frame) else {
            return RenderResult::Skipped;
        };
        let ValidatedFrame {
            dst_w,
            dst_h,
            avail_h,
            src_stride,
            src_h,
            tap_stride,
            ntsc_writes,
        } = valid;
        // `dst_h` is `avail_h + strips` by construction; `TapKey`
        // stores only `avail_h` (plus the strips), so lock the
        // identity here instead of keying a redundant field.
        debug_assert_eq!(
            dst_h,
            avail_h.saturating_add(self.reserved.height_loss(self.size.height) as usize)
        );
        // Tap coverage admits here (memoizing `&mut`), not inside the
        // pure geometry check above. Refuse before any indexer runs.
        if !self.taps_covered(dst_w, avail_h, src_stride, tap_stride, ntsc_writes) {
            warn!("softbuffer tap coverage mismatch: refusing frame");
            return RenderResult::Skipped;
        }
        let Some(surface) = self.surface.as_mut() else {
            return RenderResult::Skipped;
        };
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
        // Strip rows keep the cleared black: the surface buffer is
        // reused across frames, so stale game pixels must not linger.
        for row in buffer.as_mut()[..top.saturating_mul(dst_w)].iter_mut() {
            *row = 0;
        }
        for row in buffer.as_mut()[(top.saturating_add(avail_h)).saturating_mul(dst_w)..].iter_mut()
        {
            *row = 0;
        }

        let src = frame.as_ref();
        let game_rows = top.saturating_mul(dst_w);
        let game_len = avail_h.saturating_mul(dst_w);
        let dst = &mut buffer.as_mut()[game_rows..game_rows.saturating_add(game_len)];
        // let src_w = self.render_profile.source_logical_size.width;
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

        match frame.format() {
            PixelFormat::Rgba => {
                // Fallible indexer: `get(..)` renders opaque black on
                // short rows instead of `unwrap()` panicking the UI.
                Self::rendering(
                    dst,
                    src_stride / 4,
                    src_h,
                    dst_w,
                    avail_h,
                    |i| {
                        let base = i.saturating_mul(4);
                        src.get(base..base.saturating_add(4))
                            .and_then(|s| s.try_into().ok())
                            .unwrap_or([0, 0, 0, 255])
                    },
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
                        |i| self.ntsc_buffer.get(i).copied().unwrap_or(0).to_be_bytes(),
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
                        |i| {
                            let idx = src.get(i).copied().unwrap_or(0) as usize;
                            palette.get(idx).copied().unwrap_or(0).to_be_bytes()
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

    #[test]
    fn blend_osd_partial_alpha_uses_integer_math() {
        // Half red over black: (255*128 + 0*127 + 127) / 255 = 128.
        // Integer fixed-point with round-half-up, no FPU per pixel.
        let mut buffer = vec![0xFF00_0000u32; 1];
        let layer = nerust_render_traits::renderer::OsdLayer {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 128],
        };
        SoftbufferRenderer::blend_osd(&mut buffer, 1, 1, &layer);
        assert_eq!(buffer[0], 0xFF80_0000);
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
            |i| {
                let base = i.saturating_mul(4);
                src.get(base..base.saturating_add(4))
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or([0, 0, 0, 255])
            },
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
                |i| {
                    let base = i.saturating_mul(4);
                    src.get(base..base.saturating_add(4))
                        .and_then(|s| s.try_into().ok())
                        .unwrap_or([0, 0, 0, 255])
                },
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
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        // Second call hits the cache; same verdict, no rescan.
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
    }

    #[test]
    fn taps_cache_invalidates_on_lut_rebuild() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
        // Same-size rebuild bumps the generation: the cached verdict
        // misses on rev even though every length matches.
        let rev_before = renderer.lut.rev;
        renderer.resize_lut();
        assert!(renderer.lut.rev != rev_before);
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        assert!(renderer.taps_cache.is_some());
    }

    #[test]
    fn taps_rev_bumps_on_every_mutator() {
        let mut lut = LutEntry::new();
        let rev0 = lut.rev;
        lut.resize_lut_bilinear(2, 2, 2, 2, (1.0, 1.0));
        assert!(lut.rev != rev0);
        let rev1 = lut.rev;
        lut.resize_lut_nearest_neighbor(2, 2, 2, 2, (1.0, 1.0));
        assert!(lut.rev != rev1);
        let rev2 = lut.rev;
        lut.resize_lut(SurfaceSize::new(2, 2), 1.0, SurfaceSize::new(2, 2));
        assert!(lut.rev != rev2);
    }

    #[test]
    fn taps_cache_misses_on_rev_change() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        // Same lengths, bumped rev: verdict must rescan, not hit cache.
        renderer.lut.bump_rev();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        let (key, covered) = renderer.taps_cache.as_ref().unwrap();
        assert_eq!(key.lut_rev, renderer.lut.rev);
        assert!(*covered);
    }

    #[test]
    fn render_skips_without_surface_before_tap_scan() {
        use nerust_render_traits::{FrameBuffer, PixelFormat};
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        // No surface attached: must skip even with a valid tap key.
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        assert_eq!(renderer.render(&frame), RenderResult::Skipped);
    }

    #[test]
    fn taps_cover_rejects_short_lut() {
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        assert!(!renderer.taps_covered(8, 8, 16, 4, 0));
    }

    #[test]
    fn taps_cover_rejects_out_of_range_taps() {
        // x tap beyond the source stride.
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.lut = LutEntry {
            x_lut: vec![Some((999, 1)), None],
            y_lut: vec![None, None],
            kernel: ResizeKernel::Bilinear,
            rev: 1,
        };
        renderer.resize_buffer = vec![0; 8];
        assert!(!renderer.taps_covered(1, 1, 4, 4, 0));
        // y tap beyond the resize buffer.
        renderer.lut = LutEntry {
            x_lut: vec![Some((0, 1)), None],
            y_lut: vec![Some((99, 1)), None],
            kernel: ResizeKernel::Bilinear,
            rev: 2,
        };
        renderer.resize_buffer = vec![0; 8 * 4];
        assert!(!renderer.taps_covered(8, 1, 32, 4, 0));
        // NTSC writes beyond the NTSC buffer.
        renderer.lut = LutEntry::new();
        renderer.lut.resize_lut(
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
        renderer.resize_buffer = vec![0; 8 * 4];
        renderer.ntsc_buffer = vec![0; 10];
        assert!(!renderer.taps_covered(8, 8, 32, 4, 100));
    }

    #[test]
    fn tap_key_distinguishes_shape_and_reservation() {
        // Same dst/tap numbers but different src geometry or strips
        // must not hit the stale-true cache.
        let mut renderer = SoftbufferRenderer::new(&test_profile());
        renderer.size = SurfaceSize::new(8, 8);
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 8, 1024, 256, 0));
        let first_key = renderer.taps_cache.as_ref().unwrap().0;
        // Reserved strip change flips the key.
        renderer.reserved.top = 2;
        renderer.resize_lut();
        assert!(renderer.taps_covered(8, 6, 1024, 256, 0));
        let second_key = renderer.taps_cache.as_ref().unwrap().0;
        assert_ne!(first_key, second_key);
        assert_eq!(second_key.reserved_top, 2);
        assert_eq!(second_key.avail_h, 6);
        assert_eq!(second_key.src_w, 256);
        // Byte stride is keyed (not just tap pixels): strides sharing
        // a `tap_stride` under integer division still miss the cache.
        assert_eq!(second_key.src_stride, 1024);
    }

    #[test]
    fn fallible_rgba_indexer_renders_black_on_short_rows() {
        // Short source rows never panic: missing texels fall back to
        // opaque black instead of `unwrap()` on the UI thread.
        let short: Vec<u8> = vec![0x10, 0x20];
        let px = |i: usize| {
            let base = i.saturating_mul(4);
            short
                .get(base..base.saturating_add(4))
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0, 0, 0, 255])
        };
        assert_eq!(px(0), [0, 0, 0, 255]);
        assert_eq!(px(99), [0, 0, 0, 255]);
    }
}
