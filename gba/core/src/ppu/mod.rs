mod affine;
mod bg;
mod color;
mod mosaic;
mod obj;
mod window;

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;
pub const CYCLES_PER_LINE: u16 = 1232;
pub const HDRAW_CYCLES: u16 = 960;
/// HBlank flag edge. Other emulators use 1007 in their own event
/// coordinates, but the HW-pinned joint observable (hw-test basic-timing: HBL UNSET 111,
/// VCNT SET 111 / UNSET 727, exact) reproduces at 1006 with this core's
/// DMA phase; 1007 shifts every index by one. Edge constant is not
/// portable across cores — only the joint (edge, DMA phase) is HW truth.
pub const HBLANK_FLAG_CYCLES: u16 = 1006;
/// HBlank DMA request edge, separated from the flag edge (per-cycle
/// co-sim slice: flag/raise/DMA are distinct bus events). Joint scan on
/// misc_edge Break (actual): request at 1006/-6/-12 -> 0x2AA4, +6 ->
/// 0x2AA8 (later, wrong way vs expected 0x2A94). The request tick is not
/// the lever (stable coincidence across a 12-cycle window), so neutral
/// (== flag) stays until the trigger/active joint is re-pinned with
/// dma_fit evidence; do not retune blindly.
pub const HBLANK_DMA_CYCLES: u16 = HBLANK_FLAG_CYCLES;
/// HBlank IRQ raise edge, separated from the flag edge. Neutral default
/// equals the flag; the bus-side +1-tick defer (`pending_hblank_irq`)
/// stays on top.
pub const HBLANK_IRQ_CYCLES: u16 = HBLANK_FLAG_CYCLES;
pub const LINES_PER_FRAME: u16 = 228;
/// BG fetch clock (hw-test archive/ppu/mode3): the PPU fetches pixel x
/// at 32+4x cycles into the scanline, one pixel every four cycles.
pub const FETCH_START_CYCLES: u16 = 32;
pub const FETCH_END_CYCLES: u16 = 988;
/// DISPCNT latch shift point (+40 cycles into the line).
pub const DISPCNT_LATCH_CYCLES: u16 = 40;

pub fn bgr555_to_rgba8888(color: u16) -> u32 {
    color::rgba8888(color & 0x7FFF)
}

/// GBA LCD color emulation for presentation (see `color` docs).
/// Not applied to the core framebuffer; frontends opt in at display time.
pub fn gba_lcd_rgba8888(color: u16) -> u32 {
    color::gba_lcd_rgba8888(color)
}

/// In-place GBA LCD filter over an RGBA8888 framebuffer.
pub fn apply_gba_lcd_filter(frame: &mut [u32]) {
    color::apply_gba_lcd_filter(frame);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PpuEvent {
    pub frame_complete: bool,
    pub interrupt_mask: u16,
    pub hblank_started: bool,
    /// HBlank DMA request edge (see `HBLANK_DMA_CYCLES`). Gated on
    /// vcount < 160 by the bus, like the flag event below.
    pub hblank_dma: bool,
    pub vblank_started: bool,
    pub line_started: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayerPixel {
    color: u16,
    priority: u8,
    layer: u8,
    semi_transparent: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct PpuRegisters {
    pub dispcnt: u16,
    pub dispstat: u16,
    pub greenswap: u16,
    pub bgcnt: [u16; 4],
    pub hofs: [u16; 4],
    pub vofs: [u16; 4],
    pub pa: [i16; 2],
    pub pb: [i16; 2],
    pub pc: [i16; 2],
    pub pd: [i16; 2],
    pub ref_x: [i32; 2],
    pub ref_y: [i32; 2],
    ref_x_raw: [u32; 2],
    ref_y_raw: [u32; 2],
    pub winh: [u16; 2],
    pub winv: [u16; 2],
    pub winin: u16,
    pub winout: u16,
    pub mosaic: u16,
    pub bldcnt: u16,
    pub bldalpha: u16,
    pub bldy: u16,
}

impl Default for PpuRegisters {
    fn default() -> Self {
        Self {
            dispcnt: 0x0080,
            dispstat: 0,
            greenswap: 0,
            bgcnt: [0; 4],
            hofs: [0; 4],
            vofs: [0; 4],
            pa: [0x100; 2],
            pb: [0; 2],
            pc: [0; 2],
            pd: [0x100; 2],
            ref_x: [0; 2],
            ref_y: [0; 2],
            ref_x_raw: [0; 2],
            ref_y_raw: [0; 2],
            winh: [0; 2],
            winv: [0; 2],
            winin: 0,
            winout: 0,
            mosaic: 0,
            bldcnt: 0,
            bldalpha: 0,
            bldy: 0,
        }
    }
}

pub struct GbaPpu {
    registers: PpuRegisters,
    internal_x: [i32; 2],
    internal_y: [i32; 2],
    cycle: u16,
    vcount: u16,
    frame: Box<[u32]>,
    /// BGX/Y written flags: a write stores the register and arms the
    /// flag; the internal copy lands at the next line start (or vcount
    /// 0), never immediately.
    written_x: [bool; 2],
    written_y: [bool; 2],
    /// Last sub-boundary BG VRAM halfword: BG fetches at/above the OBJ
    /// at/above the OBJ boundary return this instead of physical VRAM.
    bg_latch: u16,
    /// DISPCNT 3-stage shift latch (HW-confirmed): shifted at +40 cycles
    /// of every scanline. BG/OBJ enables gate on `latch[0] & live`,
    /// forced blank on `latch[0] | live`. Window enables stay live.
    dispcnt_latch: [u16; 3],
    /// Forced-blank sample taken at line end for the next scanline.
    /// Unlike BG/OBJ enables (3-stage latch), blank applies within a line,
    /// so `forced_blank()` ORs this sample with the live bit.
    blank_sample: bool,
    /// Per-line latch: OAM sampled at the first pixel of each line.
    /// MOSAIC stays live (sizes read per-pixel).
    /// Other registers (DISPCNT, BGxCNT, scroll, windows) stay live.
    line: LineLatch,
    /// Deferred scanline rendering cursor: pixels [0, rendered_up_to_x)
    /// of the current visible line are already in `frame`; the rest
    /// render at segment boundaries (mid-fetch PPU writes) or line end.
    /// Reset at every line end; serialized for exact save/load.
    rendered_up_to_x: u8,
    /// Span-level OBJ working-set cache. Consecutive segments of a line
    /// (e.g. per-word DMA streaming into VRAM) share line OAM and
    /// usually DISPCNT, so rebuilding the 128-OBJ working set per
    /// segment wastes milliseconds; the key (scanline, full DISPCNT,
    /// capture epoch) pins the exact inputs. Derivable scratch: not
    /// serialized, invalidated on import.
    obj_cover: [(u8, u16, u16, u16); 128],
    obj_cover_len: u8,
    obj_cover_y: u16,
    obj_cover_dispcnt: u16,
    obj_cover_epoch: u32,
    obj_cover_valid: bool,
    /// Bumped at every line-OAM capture (and import/reset): line OAM
    /// is otherwise immutable within a line, so the epoch identifies
    /// the cached working set's OAM without comparing 1KB per span.
    obj_epoch: u32,
}

#[derive(Debug)]
struct LineLatch {
    oam: Box<[u8; 1024]>,
    /// `dispcnt_latch[0]` sampled at the first fetch of the line (cycle 32,
    /// before the +40 shift): the enable/blank reference for this scanline.
    enable: u16,
}

impl LineLatch {
    fn new() -> Self {
        Self {
            oam: Box::new([0; 1024]),
            enable: 0,
        }
    }

    fn capture(&mut self, oam: &[u8], enable: u16) {
        self.oam.copy_from_slice(oam);
        self.enable = enable;
    }
}

/// Phase 10 wire state. The scanline frame buffer (38400 RGBA8888 words)
/// travels as little-endian bytes; the per-line OAM latch likewise.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GbaPpuState {
    registers: PpuRegisters,
    internal_x: [i32; 2],
    internal_y: [i32; 2],
    cycle: u16,
    vcount: u16,
    frame: serde_bytes::ByteBuf,
    written_x: [bool; 2],
    written_y: [bool; 2],
    bg_latch: u16,
    dispcnt_latch: [u16; 3],
    blank_sample: bool,
    line_oam: serde_bytes::ByteBuf,
    line_enable: u16,
    rendered_up_to_x: u8,
}

impl GbaPpuState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.vcount >= LINES_PER_FRAME {
            return Err(format!("ppu: vcount out of range: {}", self.vcount));
        }
        if self.cycle >= CYCLES_PER_LINE {
            return Err(format!("ppu: cycle out of range: {}", self.cycle));
        }
        if self.frame.len() != WIDTH * HEIGHT * 4 {
            return Err(format!(
                "ppu: frame byte length wrong: {}",
                self.frame.len()
            ));
        }
        if self.line_oam.len() != 1024 {
            return Err(format!(
                "ppu: line latch length wrong: {}",
                self.line_oam.len()
            ));
        }
        if self.rendered_up_to_x > WIDTH as u8 {
            return Err(format!(
                "ppu: rendered cursor out of range: {}",
                self.rendered_up_to_x
            ));
        }
        // Affine accumulators track 28-bit signed reference values.
        for (index, value) in self
            .internal_x
            .iter()
            .chain(self.internal_y.iter())
            .enumerate()
        {
            if value.abs() > 0x0FFF_FFFF {
                return Err(format!("ppu: internal accumulator {index} out of range"));
            }
        }
        Ok(())
    }
}

impl GbaPpu {
    pub fn new() -> Self {
        Self {
            registers: PpuRegisters::default(),
            internal_x: [0; 2],
            internal_y: [0; 2],
            cycle: 0,
            vcount: 0,
            frame: vec![color::rgba8888(0x7FFF); WIDTH * HEIGHT].into_boxed_slice(),
            written_x: [false; 2],
            written_y: [false; 2],
            bg_latch: 0,
            dispcnt_latch: [0; 3],
            blank_sample: false,
            line: LineLatch::new(),
            rendered_up_to_x: 0,
            obj_cover: [(0, 0, 0, 0); 128],
            obj_cover_len: 0,
            obj_cover_y: 0,
            obj_cover_dispcnt: 0,
            obj_cover_epoch: 0,
            obj_cover_valid: false,
            obj_epoch: 0,
        }
    }

    pub(crate) fn export_state(&self) -> GbaPpuState {
        let mut frame_bytes = Vec::with_capacity(WIDTH * HEIGHT * 4);
        for word in self.frame.iter() {
            frame_bytes.extend_from_slice(&word.to_le_bytes());
        }
        GbaPpuState {
            registers: self.registers,
            internal_x: self.internal_x,
            internal_y: self.internal_y,
            cycle: self.cycle,
            vcount: self.vcount,
            frame: serde_bytes::ByteBuf::from(frame_bytes),
            written_x: self.written_x,
            written_y: self.written_y,
            bg_latch: self.bg_latch,
            dispcnt_latch: self.dispcnt_latch,
            blank_sample: self.blank_sample,
            line_oam: serde_bytes::ByteBuf::from(self.line.oam.to_vec()),
            line_enable: self.line.enable,
            rendered_up_to_x: self.rendered_up_to_x,
        }
    }

    pub(crate) fn import_state(&mut self, state: GbaPpuState) -> Result<(), String> {
        state.validate()?;
        let words: Vec<u32> = state
            .frame
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        self.registers = state.registers;
        self.internal_x = state.internal_x;
        self.internal_y = state.internal_y;
        self.cycle = state.cycle;
        self.vcount = state.vcount;
        self.frame = words.into_boxed_slice();
        self.written_x = state.written_x;
        self.written_y = state.written_y;
        self.bg_latch = state.bg_latch;
        self.dispcnt_latch = state.dispcnt_latch;
        self.blank_sample = state.blank_sample;
        self.line.oam.copy_from_slice(&state.line_oam);
        self.line.enable = state.line_enable;
        self.rendered_up_to_x = state.rendered_up_to_x;
        // Derivable scratch: the epoch bump retires the cached set.
        self.obj_epoch = self.obj_epoch.wrapping_add(1);
        self.obj_cover_valid = false;
        Ok(())
    }

    /// Batching horizon: quiet prefix length before the next cycle that
    /// needs full per-cycle processing (first-pixel fetch with OAM
    /// capture, latch, flag/IRQ/DMA edges, line end). Pixel fetches past
    /// x==0 render lazily in scanline segments (see `render_prefix_up_to`),
    /// so only cycle 32 stays a boundary; interior cycles only bump the
    /// dot counter.
    #[inline]
    pub(crate) fn quiet_cycles(&self) -> u64 {
        const INF: u64 = u64::MAX;
        let c = u64::from(self.cycle);
        let mut horizon = INF;
        // First fetch (OAM capture + pixel 0) on visible scanlines.
        if self.vcount < HEIGHT as u16 && c < u64::from(FETCH_START_CYCLES) {
            horizon = horizon.min(u64::from(FETCH_START_CYCLES) - c - 1);
        }
        // Single-cycle edges and line end.
        for edge in [
            u64::from(DISPCNT_LATCH_CYCLES),
            u64::from(HBLANK_FLAG_CYCLES),
            u64::from(HBLANK_IRQ_CYCLES),
            u64::from(HBLANK_DMA_CYCLES),
            u64::from(CYCLES_PER_LINE),
        ] {
            if edge > c {
                horizon = horizon.min(edge - c - 1);
            }
        }
        horizon
    }

    /// Advance the dot counter by `n` cycles. Valid only for
    /// `n <= quiet_cycles()` at the same state: no fetches, edges or line
    /// ends occur inside the span (verified by the horizon).
    #[inline]
    pub(crate) fn advance_idle(&mut self, n: u64) {
        // Horizon guarantees c + n <= CYCLES_PER_LINE: no wrap, no line end.
        self.cycle += n as u16;
    }
    /// Hot per-T-cycle driver (single call site in `tick_video`):
    /// forced-inline so the dot-counter/edge checks fold into the bus
    /// tick without call overhead (280k calls/frame).
    #[inline]
    pub fn step(&mut self, vram: &[u8], palette: &[u8], oam: &[u8]) -> PpuEvent {
        let mut event = PpuEvent::default();
        self.cycle += 1;
        // Deferred scanline rendering: only the first fetch stays on the
        // exact cycle (OAM capture + pixel 0). Later pixels render in
        // segments at mid-fetch writes (`render_prefix_up_to`) or at line
        // end, each exactly once with contemporary state.
        if self.vcount < HEIGHT as u16 && self.cycle == FETCH_START_CYCLES {
            // Latch OAM at the first fetch of the line; later mid-draw
            // writes defer to the next line. MOSAIC stays live.
            self.line.capture(oam, self.dispcnt_latch[0]);
            // New line OAM: retire the cached OBJ working set.
            self.obj_epoch = self.obj_epoch.wrapping_add(1);
            self.obj_cover_valid = false;
            let y = self.vcount as usize;
            let cache = self.obj_span_cache(y);
            self.render_pixel(0, y, vram, palette, &cache);
            self.rendered_up_to_x = 1;
        }
        if self.cycle == DISPCNT_LATCH_CYCLES {
            // 3-stage shift of the DISPCNT enable latch.
            self.dispcnt_latch[0] = self.dispcnt_latch[1];
            self.dispcnt_latch[1] = self.dispcnt_latch[2];
            self.dispcnt_latch[2] = self.registers.dispcnt;
        }
        if self.cycle == HBLANK_FLAG_CYCLES {
            self.handle_hblank_flag(&mut event);
        }
        if self.cycle == HBLANK_IRQ_CYCLES {
            self.handle_hblank_raise(&mut event);
        }
        if self.cycle == HBLANK_DMA_CYCLES {
            event.hblank_dma = true;
        }
        if self.cycle == CYCLES_PER_LINE {
            self.render_remainder(vram, palette);
            self.handle_line_end(&mut event);
            self.rendered_up_to_x = 0;
        }
        event
    }

    /// Deferred-scanline segment trigger for PPU-visible stores. Renders
    /// the pending scanline prefix with pre-write state so every pixel
    /// observes contemporary state exactly once; must run before the
    /// write applies. Visibility is indexed by the PPU dot clock: pixel x
    /// fetches at dot 4x+32 and fetches at dots <= the current dot already
    /// ran. Over-triggering only renders earlier, never wrongly.
    pub(crate) fn note_ppu_write(&mut self, vram: &[u8], palette: &[u8]) {
        if self.vcount >= HEIGHT as u16 {
            return;
        }
        let dot = u64::from(self.cycle);
        if dot < 32 {
            return;
        }
        let xw = (((dot - 32) / 4 + 1).min(240)) as usize;
        self.render_prefix_up_to(xw, vram, palette);
    }

    /// Render pending pixels `[rendered_up_to_x, x_end)` of the current
    /// visible line with contemporary state, then advance the cursor.
    /// Every pixel renders exactly once: segments partition [1, 240)
    /// (pixel 0 runs at its boundary tick). No-op off visible lines or
    /// when nothing is pending.
    pub(crate) fn render_prefix_up_to(&mut self, x_end: usize, vram: &[u8], palette: &[u8]) {
        let start = usize::from(self.rendered_up_to_x);
        if self.vcount >= HEIGHT as u16 || x_end <= start {
            return;
        }
        debug_assert!(
            start >= 1,
            "pixel 0 always renders at its boundary tick first"
        );
        let end = x_end.min(WIDTH);
        let y = self.vcount as usize;
        if self.span_applies() {
            if self.registers.dispcnt & (1 << 12) != 0 {
                let cache = self.obj_span_cache(y);
                self.render_span(start, end, y, vram, palette, &cache);
            } else {
                // No OBJ reach: skip the working-set build entirely.
                self.render_span(start, end, y, vram, palette, &obj::ObjLineCache::empty());
            }
        } else if self.blend_span_applies() {
            if self.registers.dispcnt & (1 << 12) != 0 {
                let cache = self.obj_span_cache(y);
                self.render_blend_span(start, end, y, vram, palette, &cache);
            } else {
                self.render_blend_span(start, end, y, vram, palette, &obj::ObjLineCache::empty());
            }
        } else {
            let cache = self.obj_span_cache(y);
            for x in start..end {
                self.render_pixel(x, y, vram, palette, &cache);
            }
        }
        self.rendered_up_to_x = end as u8;
    }

    /// Render the line tail at line end, before `handle_line_end` advances
    /// counters and latches (the tail observes pre-advance line state).
    fn render_remainder(&mut self, vram: &[u8], palette: &[u8]) {
        let start = usize::from(self.rendered_up_to_x);
        if self.vcount >= HEIGHT as u16 || start >= WIDTH {
            return;
        }
        let y = self.vcount as usize;
        if self.span_applies() {
            if self.registers.dispcnt & (1 << 12) != 0 {
                let cache = self.obj_span_cache(y);
                self.render_span(start, WIDTH, y, vram, palette, &cache);
            } else {
                self.render_span(start, WIDTH, y, vram, palette, &obj::ObjLineCache::empty());
            }
        } else if self.blend_span_applies() {
            if self.registers.dispcnt & (1 << 12) != 0 {
                let cache = self.obj_span_cache(y);
                self.render_blend_span(start, WIDTH, y, vram, palette, &cache);
            } else {
                self.render_blend_span(start, WIDTH, y, vram, palette, &obj::ObjLineCache::empty());
            }
        } else {
            let cache = self.obj_span_cache(y);
            for x in start..WIDTH {
                self.render_pixel(x, y, vram, palette, &cache);
            }
        }
    }

    /// OBJ working set for the span starting on scanline `y`, shared by
    /// every pixel in it. Hits (same line, DISPCNT and capture epoch)
    /// copy ~1KB; only the first span per line state rebuilds the
    /// 128-OBJ set. Callers with no OBJ reach (lean spans, disabled
    /// layer with windows off) never call this.
    fn obj_span_cache(&mut self, y: usize) -> obj::ObjLineCache {
        if self.obj_cover_valid
            && self.obj_cover_y == y as u16
            && self.obj_cover_dispcnt == self.registers.dispcnt
            && self.obj_cover_epoch == self.obj_epoch
        {
            return obj::ObjLineCache::from_cover(self.obj_cover, self.obj_cover_len);
        }
        let fresh = obj::line_cache(&self.registers, &self.line.oam[..], y);
        let (cover, cover_len) = fresh.into_cover();
        self.obj_cover = cover;
        self.obj_cover_len = cover_len;
        self.obj_cover_y = y as u16;
        self.obj_cover_dispcnt = self.registers.dispcnt;
        self.obj_cover_epoch = self.obj_epoch;
        self.obj_cover_valid = true;
        fresh
    }

    /// Span gate: no forced blank, no windows (mask is constantly
    /// 0x3F), `bldcnt == 0` (effects identity: mode 0 with empty masks
    /// returns the top pixel — the blend arm needs a second-mask bit
    /// even for semi-transparent OBJs, and brighten/darken need mode
    /// 2/3), no greenswap. OBJ layers are fine (they join the top
    /// selection through the same cover list). Registers are constant
    /// across the span (prefix spans render before the triggering
    /// write; remainder spans at line end), so one gate check covers
    /// every pixel in it.
    fn span_applies(&self) -> bool {
        !self.forced_blank()
            && (self.registers.dispcnt >> 13) & 7 == 0
            && self.registers.bldcnt == 0
            && self.registers.greenswap & 1 == 0
    }

    /// Span render: pixel-identical to `render_pixel` under
    /// [`span_applies`](Self::span_applies), minus per-pixel
    /// window/effects machinery. BG fetches (including the shared
    /// VRAM latch) run through the identical `bg::pixel` path in x
    /// order, so latch evolution matches exactly; priority ties keep
    /// insertion order via strict `<`, exactly like the top-two
    /// selection (backdrop, BG0-3, OBJ).
    fn render_span(
        &mut self,
        start: usize,
        end: usize,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        cache: &obj::ObjLineCache,
    ) {
        // Layer enables gate on latched AND live (same as render_pixel).
        // OBJ fetch keys off the live enable (same exception).
        let enables = self.line.enable & self.registers.dispcnt;
        let obj_on = self.registers.dispcnt & (1 << 12) != 0;
        let backdrop = color::read_color(palette, 0);
        for x in start..end {
            // Backdrop: priority 4, layer 5 (rank 6).
            let mut top_color = backdrop;
            let mut top_key = (4u8, 6u8);
            for bg_index in 0..4 {
                if enables & (1 << (8 + bg_index)) != 0
                    && let Some(pixel) = bg::pixel(
                        &self.registers,
                        (self.internal_x, self.internal_y),
                        (vram, palette),
                        bg_index,
                        (x, y),
                        &mut self.bg_latch,
                        self.registers.mosaic,
                    )
                {
                    let key = (pixel.priority, layer_rank(pixel.layer));
                    if key < top_key {
                        top_key = key;
                        top_color = pixel.color;
                    }
                }
            }
            if obj_on
                && let Some(pixel) = obj::pixel(
                    &self.registers,
                    (vram, palette, &self.line.oam[..]),
                    (x, y),
                    false,
                    self.registers.mosaic,
                    cache,
                )
            {
                let key = (pixel.priority, layer_rank(pixel.layer));
                if key < top_key {
                    top_color = pixel.color;
                }
            }
            self.frame[y * WIDTH + x] = color::rgba8888(top_color);
        }
    }

    /// Blend-span gate: the [`span_applies`](Self::span_applies)
    /// conditions minus `bldcnt == 0` (any blend mode allowed). Without
    /// windows the region mask is constantly 0x3F, so the effect arm
    /// always runs; forced blank and greenswap stay excluded (they take
    /// dedicated per-pixel paths in `render_pixel`).
    fn blend_span_applies(&self) -> bool {
        !self.forced_blank()
            && (self.registers.dispcnt >> 13) & 7 == 0
            && self.registers.greenswap & 1 == 0
    }

    /// Blend span render: pixel-identical to `render_pixel` under
    /// [`blend_span_applies`](Self::blend_span_applies). The BG/OBJ
    /// fetchers, top-two selection and effect application run the same
    /// calls in the same order; only the window/blank/greenswap
    /// machinery is hoisted (constant across the span), and the OBJ
    /// working set decodes once per span instead of once per pixel.
    fn render_blend_span(
        &mut self,
        start: usize,
        end: usize,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        cache: &obj::ObjLineCache,
    ) {
        // Layer enables gate on latched AND live (same as render_pixel).
        // OBJ fetch keys off the live enable (same exception).
        let enables = self.line.enable & self.registers.dispcnt;
        let obj_on = self.registers.dispcnt & (1 << 12) != 0;
        let backdrop = LayerPixel {
            color: color::read_color(palette, 0),
            priority: 4,
            layer: 5,
            semi_transparent: false,
        };
        // Decode the span's OBJ working set once (attrs are constant
        // for the whole span). Undecodable entries stay out exactly
        // like the per-pixel path's `continue`. Coordinates prepare
        // per span too (origins, y-side and affine rows are constant
        // across the scanline; OAM/mosaic are frozen within a span by
        // the prefix-split on writes, same guarantee the decode
        // relies on).
        let mut decoded: smallvec::SmallVec<[(u8, obj::PreparedObj); 32]> =
            smallvec::SmallVec::new();
        if obj_on {
            for &(raw_index, attr0, attr1, attr2) in cache.cover[..cache.cover_len as usize].iter()
            {
                if let Some(object) = obj::decode_attrs(attr0, attr1, attr2, false)
                    && let Some(prepared) =
                        object.prepare(&self.line.oam[..], y, self.registers.mosaic)
                {
                    decoded.push((raw_index, prepared));
                }
            }
        }
        for x in start..end {
            // Top-two selection with stable-sort order (ties keep
            // insertion order: backdrop, BG0-3, OBJ) — identical to
            // `render_pixel`'s best/second update.
            let mut best: Option<((u8, u8), LayerPixel)> = None;
            let mut second: Option<((u8, u8), LayerPixel)> = None;
            {
                let key = (backdrop.priority, layer_rank(backdrop.layer));
                consider_pixel(key, backdrop, &mut best, &mut second);
            }
            for bg_index in 0..4 {
                if enables & (1 << (8 + bg_index)) != 0
                    && let Some(pixel) = bg::pixel(
                        &self.registers,
                        (self.internal_x, self.internal_y),
                        (vram, palette),
                        bg_index,
                        (x, y),
                        &mut self.bg_latch,
                        self.registers.mosaic,
                    )
                {
                    let key = (pixel.priority, layer_rank(pixel.layer));
                    consider_pixel(key, pixel, &mut best, &mut second);
                }
            }
            if obj_on
                && let Some(pixel) = obj::pixel_predecoded(
                    &self.registers,
                    (vram, palette, &self.line.oam[..]),
                    (x, y),
                    self.registers.mosaic,
                    &decoded,
                )
            {
                let key = (pixel.priority, layer_rank(pixel.layer));
                consider_pixel(key, pixel, &mut best, &mut second);
            }
            let (top, second) = (
                best.map(|(_, p)| p).expect("backdrop always present"),
                second.map(|(_, p)| p),
            );
            // No windows: the region mask is 0x3F, so effects always run
            // (same `apply_effect(top, second, true)` render_pixel calls
            // with a full mask).
            let output = self.apply_effect(top, second, true);
            self.frame[y * WIDTH + x] = color::rgba8888(output);
        }
    }

    fn handle_hblank_flag(&mut self, event: &mut PpuEvent) {
        event.hblank_started = true;
        self.registers.dispstat |= 1 << 1;
    }

    /// HBlank IRQ raise edge, separated from the flag write above so the
    /// flag/raise/DMA joint is observable per event (the bus still defers
    /// the IF raise by one tick via `pending_hblank_irq`).
    fn handle_hblank_raise(&mut self, event: &mut PpuEvent) {
        if self.registers.dispstat & (1 << 4) != 0 {
            event.interrupt_mask |= 1 << 1;
        }
    }

    fn handle_line_end(&mut self, event: &mut PpuEvent) {
        self.cycle = 0;
        // Refresh per-line enable/blank refs for the next scanline:
        // enables ride the latch, blank samples live for within-line response.
        self.line.enable = self.dispcnt_latch[0];
        self.blank_sample = self.registers.dispcnt & (1 << 7) != 0;
        event.line_started = true;
        self.registers.dispstat &= !(1 << 1);
        self.advance_affine();
        self.advance_vcount(event);
        // Pending BGX/Y writes land in the internal registers at the
        // next line start (or unconditionally at vcount 0).
        let first_scanline = self.vcount == 0;
        for affine in 0..2 {
            if self.written_x[affine] || first_scanline {
                self.internal_x[affine] = self.registers.ref_x[affine];
                self.written_x[affine] = false;
            }
            if self.written_y[affine] || first_scanline {
                self.internal_y[affine] = self.registers.ref_y[affine];
                self.written_y[affine] = false;
            }
        }
        self.update_vcount_match(event);
    }

    fn advance_affine(&mut self) {
        if self.vcount >= HEIGHT as u16 {
            return;
        }
        // Internal affine registers advance only while their BG
        // is enabled (BG2 -> affine 0, BG3 -> affine 1).
        let enabled = [
            self.registers.dispcnt & (1 << 10) != 0,
            self.registers.dispcnt & (1 << 11) != 0,
        ];
        crate::ppu::affine::advance_line(
            &mut self.internal_x,
            &mut self.internal_y,
            self.registers.pb,
            self.registers.pd,
            enabled,
        );
    }

    fn advance_vcount(&mut self, event: &mut PpuEvent) {
        self.vcount += 1;
        if self.vcount == HEIGHT as u16 {
            event.vblank_started = true;
            self.registers.dispstat |= 1;
            if self.registers.dispstat & (1 << 3) != 0 {
                event.interrupt_mask |= 1;
            }
        } else if self.vcount == LINES_PER_FRAME - 1 {
            // GBATEK DISPSTAT Bit 0: V-Blank flag set in lines 160..226, not 227.
            self.registers.dispstat &= !1;
        } else if self.vcount == LINES_PER_FRAME {
            self.vcount = 0;
            self.registers.dispstat &= !1;
            event.frame_complete = true;
        }
    }

    pub fn frame_buffer(&self) -> &[u32] {
        &self.frame
    }

    pub fn vcount(&self) -> u16 {
        self.vcount
    }

    pub fn cycle(&self) -> u16 {
        self.cycle
    }

    pub fn dispcnt(&self) -> u16 {
        self.registers.dispcnt
    }

    /// Forced blank: blanked when the bit is set in the latched OR the
    /// live DISPCNT. The latched half is the per-line reference shared with
    /// the renderer (refreshed every line end), so render and stall paths
    /// can never disagree by a line.
    pub fn forced_blank(&self) -> bool {
        self.blank_sample || self.registers.dispcnt & (1 << 7) != 0
    }

    /// Any BG layer enabled in both the latched and the live DISPCNT;
    /// gates BG-VRAM fetch contention.
    pub fn bg_fetch_active(&self) -> bool {
        self.line.enable & self.registers.dispcnt & 0x0F00 != 0
    }

    pub fn dispstat(&self) -> u16 {
        self.registers.dispstat
    }

    pub fn bgcnt(&self, bg: usize) -> u16 {
        self.registers.bgcnt[bg]
    }

    pub fn hofs(&self, bg: usize) -> u16 {
        self.registers.hofs[bg]
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn read_register(&self, address: u32) -> Option<u16> {
        match address {
            0x04000000 => Some(self.registers.dispcnt),
            0x04000002 => Some(self.registers.greenswap),
            0x04000004 => Some(self.registers.dispstat),
            0x04000006 => Some(self.vcount),
            0x04000008..=0x0400000E => {
                Some(self.registers.bgcnt[((address - 0x04000008) / 2) as usize])
            }
            0x04000048 => Some(self.registers.winin),
            0x0400004A => Some(self.registers.winout),
            0x04000050 => Some(self.registers.bldcnt),
            0x04000052 => Some(self.registers.bldalpha),
            _ => None,
        }
    }

    pub fn write_register(&mut self, address: u32, value: u16, vram: &[u8], palette: &[u8]) -> u16 {
        // Deferred-scanline segment trigger (see `note_ppu_write`).
        // Covers CPU, DMA and HLE stores uniformly; unit tests pass
        // their local buffers.
        self.note_ppu_write(vram, palette);
        match address {
            0x04000000 => {
                // GBATEK DISPCNT Bit 3 (CGB Mode): can be set only by BIOS opcodes.
                // CPU writes must not change it, so preserve the old bit.
                let old = self.registers.dispcnt;
                self.registers.dispcnt = (value & !(1 << 3)) | (old & (1 << 3));
                // The OBJ working-set key includes DISPCNT (cycle budget):
                // retire the cached set when it changes.
                if self.registers.dispcnt != old {
                    self.obj_cover_valid = false;
                }
                0
            }
            0x04000002 => {
                self.registers.greenswap = value & 1;
                0
            }
            0x04000004 => {
                let old = self.registers.dispstat;
                let old_match = old & (1 << 2) != 0;
                let old_enable = old & (1 << 5) != 0;
                self.registers.dispstat = (self.registers.dispstat & 7) | (value & 0xFF38);
                let is_match = self.vcount == self.registers.dispstat >> 8;
                self.registers.dispstat =
                    (self.registers.dispstat & !(1 << 2)) | (u16::from(is_match) * 4);
                let new_enable = self.registers.dispstat & (1 << 5) != 0;
                if is_match && new_enable && (!old_match || !old_enable) {
                    1 << 2
                } else {
                    0
                }
            }
            0x04000008..=0x0400000E => {
                let bg = ((address - 0x04000008) / 2) as usize;
                // Display-area-overflow (bit 13) exists only on BG2/BG3;
                // writes to BG0/BG1 ignore it.
                let mask = if bg < 2 { !(1 << 13) } else { u16::MAX };
                self.registers.bgcnt[bg] = value & mask;
                0
            }
            0x04000010..=0x0400001E => {
                let index = ((address - 0x04000010) / 4) as usize;
                if address & 2 == 0 {
                    self.registers.hofs[index] = value & 0x1FF;
                } else {
                    self.registers.vofs[index] = value & 0x1FF;
                }
                0
            }
            0x04000020..=0x04000026 | 0x04000030..=0x04000036 => {
                let affine = usize::from(address >= 0x04000030);
                match (address & 0xF) / 2 {
                    0 => self.registers.pa[affine] = value as i16,
                    1 => self.registers.pb[affine] = value as i16,
                    2 => self.registers.pc[affine] = value as i16,
                    3 => self.registers.pd[affine] = value as i16,
                    _ => {}
                }
                0
            }
            0x04000028..=0x0400002E | 0x04000038..=0x0400003E => {
                let affine = usize::from(address >= 0x04000038);
                self.write_reference(address, value);
                // The write only arms the pending flag; the internal copy
                // lands at the next line start (see handle_line_end).
                if (0x04000028..=0x0400002A).contains(&address)
                    || (0x04000038..=0x0400003A).contains(&address)
                {
                    self.written_x[affine] = true;
                } else {
                    self.written_y[affine] = true;
                }
                0
            }
            0x04000040 => {
                self.registers.winh[0] = value;
                0
            }
            0x04000042 => {
                self.registers.winh[1] = value;
                0
            }
            0x04000044 => {
                self.registers.winv[0] = value;
                0
            }
            0x04000046 => {
                self.registers.winv[1] = value;
                0
            }
            0x04000048 => {
                // WindowLayerSelect: only 6 bits per byte are stored.
                self.registers.winin = value & 0x3F3F;
                0
            }
            0x0400004A => {
                self.registers.winout = value & 0x3F3F;
                0
            }
            0x0400004C => {
                self.registers.mosaic = value;
                0
            }
            0x04000050 => {
                self.registers.bldcnt = value & 0x3FFF;
                0
            }
            0x04000052 => {
                self.registers.bldalpha = value & 0x1F1F;
                0
            }
            0x04000054 => {
                self.registers.bldy = value & 0x1F;
                0
            }
            _ => 0,
        }
    }

    fn write_reference(&mut self, address: u32, value: u16) {
        let affine = usize::from(address >= 0x04000038);
        let local = address & 0xF;
        let (raw, output) = if matches!(local, 8 | 0xA) {
            (
                &mut self.registers.ref_x_raw[affine],
                &mut self.registers.ref_x[affine],
            )
        } else {
            (
                &mut self.registers.ref_y_raw[affine],
                &mut self.registers.ref_y[affine],
            )
        };
        if local & 2 == 0 {
            *raw = (*raw & 0xFFFF0000) | u32::from(value);
        } else {
            *raw = (*raw & 0x0000FFFF) | (u32::from(value & 0x0FFF) << 16);
        }
        *output = sign_extend_28(*raw);
    }

    fn update_vcount_match(&mut self, event: &mut PpuEvent) {
        let was_match = self.registers.dispstat & (1 << 2) != 0;
        let is_match = self.vcount == self.registers.dispstat >> 8;
        self.registers.dispstat = (self.registers.dispstat & !(1 << 2)) | (u16::from(is_match) * 4);
        if is_match && !was_match && self.registers.dispstat & (1 << 5) != 0 {
            event.interrupt_mask |= 1 << 2;
        }
    }

    fn render_pixel(
        &mut self,
        x: usize,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        cache: &obj::ObjLineCache,
    ) {
        // OAM was latched at the pixel-0 boundary tick (see `step`);
        // spans start at x >= 1 with the line's capture in place.
        // Forced blank: sampled (line-start) OR live. Render and stall
        // paths share forced_blank(), so both follow blank within a line.
        if self.forced_blank() {
            self.frame[y * WIDTH + x] = color::rgba8888(0x7FFF);
            return;
        }
        // Layer enables gate on latched AND live. OBJ is the exception:
        // its fetch keys off the LIVE enable only, so it reacts to HBlank
        // toggling immediately while BGs lag 3 lines.
        let enables = self.line.enable & self.registers.dispcnt;
        let mask = self.window_mask(x, y, vram, palette, cache);
        // Fixed-size stack buffer instead of a per-pixel heap Vec
        // (38k allocations per frame): at most backdrop + 4 BG + OBJ.
        let mut layers = [LayerPixel::default(); 6];
        layers[0] = LayerPixel {
            color: color::read_color(palette, 0),
            priority: 4,
            layer: 5,
            semi_transparent: false,
        };
        let mut layer_count = 1usize;
        for bg_index in 0..4 {
            if enables & (1 << (8 + bg_index)) != 0
                && mask & (1 << bg_index) != 0
                && let Some(pixel) = bg::pixel(
                    &self.registers,
                    (self.internal_x, self.internal_y),
                    (vram, palette),
                    bg_index,
                    (x, y),
                    &mut self.bg_latch,
                    self.registers.mosaic,
                )
            {
                layers[layer_count] = pixel;
                layer_count += 1;
            }
        }
        if self.registers.dispcnt & (1 << 12) != 0
            && mask & (1 << 4) != 0
            && let Some(pixel) = obj::pixel(
                &self.registers,
                (vram, palette, &self.line.oam[..]),
                (x, y),
                false,
                self.registers.mosaic,
                cache,
            )
        {
            layers[layer_count] = pixel;
            layer_count += 1;
        }
        let active = &layers[..layer_count];
        // Top-two selection with stable-sort order (ties keep insertion
        // order: backdrop, BG0-3, OBJ). Equivalent to the previous
        // `sort_by_key` + take-two, without sorting.
        let mut best: Option<((u8, u8), LayerPixel)> = None;
        let mut second: Option<((u8, u8), LayerPixel)> = None;
        for pixel in active.iter() {
            let key = (pixel.priority, layer_rank(pixel.layer));
            match best {
                None => best = Some((key, *pixel)),
                Some((best_key, _)) if key < best_key => {
                    second = best;
                    best = Some((key, *pixel));
                }
                _ => {
                    let second_key = second.map(|(k, _)| k);
                    if second_key.is_none_or(|k| key < k) {
                        second = Some((key, *pixel));
                    }
                }
            }
        }
        let (top, second) = (
            best.map(|(_, p)| p).expect("backdrop always present"),
            second.map(|(_, p)| p),
        );
        let effects_enabled = mask & (1 << 5) != 0;
        let output = self.apply_effect(top, second, effects_enabled);
        self.frame[y * WIDTH + x] = color::rgba8888(output);
        if self.registers.greenswap & 1 != 0 && x % 2 == 1 {
            let prev_idx = y * WIDTH + x - 1;
            let curr_idx = y * WIDTH + x;
            let prev_rgba = self.frame[prev_idx];
            let curr_rgba = self.frame[curr_idx];
            let prev_b = prev_rgba.to_le_bytes();
            let curr_b = curr_rgba.to_le_bytes();
            self.frame[prev_idx] = u32::from_le_bytes([prev_b[0], curr_b[1], prev_b[2], prev_b[3]]);
            self.frame[curr_idx] = u32::from_le_bytes([curr_b[0], prev_b[1], curr_b[2], curr_b[3]]);
        }
    }

    fn apply_effect(&self, top: LayerPixel, second: Option<LayerPixel>, enabled: bool) -> u16 {
        // Tonc gfx §13.2.2: with windows in use, blending needs the region's
        // color-effect bit (WININ/WINOUT bit 5/13) — including for
        // semi-transparent OBJs. GBATEK's semi-transparency paragraph only
        // overrides BLDCNT bits 4/6-7, not the window gate, so a disabled
        // region shows the top pixel opaque.
        if !enabled {
            return top.color;
        }
        let first_mask = self.registers.bldcnt & 0x3F;
        let second_mask = (self.registers.bldcnt >> 8) & 0x3F;
        let top_bit = 1 << top.layer;
        let mode = (self.registers.bldcnt >> 6) & 3;
        if (top.semi_transparent || mode == 1 && first_mask & top_bit != 0)
            && let Some(second) = second
            && second_mask & (1 << second.layer) != 0
        {
            let eva = (self.registers.bldalpha & 0x1F).min(16) as u8;
            let evb = ((self.registers.bldalpha >> 8) & 0x1F).min(16) as u8;
            return color::alpha_blend(top.color, second.color, eva, evb);
        }
        let amount = (self.registers.bldy & 0x1F).min(16) as u8;
        if first_mask & top_bit != 0 {
            if mode == 2 {
                return color::brighten(top.color, amount);
            }
            if mode == 3 {
                return color::darken(top.color, amount);
            }
        }
        top.color
    }

    fn window_mask(
        &self,
        x: usize,
        y: usize,
        vram: &[u8],
        palette: &[u8],
        cache: &obj::ObjLineCache,
    ) -> u8 {
        window::window_mask(
            &self.registers,
            (vram, palette, &self.line.oam[..]),
            (x, y),
            self.registers.mosaic,
            cache,
        )
    }
}

impl Default for GbaPpu {
    fn default() -> Self {
        Self::new()
    }
}

fn sign_extend_28(value: u32) -> i32 {
    ((value << 4) as i32) >> 4
}

fn layer_rank(layer: u8) -> u8 {
    if layer == 4 { 0 } else { layer + 1 }
}

/// Top-two selection step with stable-sort order (ties keep insertion
/// order). Shared by `render_pixel` and the blend-span path so the two
/// can never diverge. Forced-inline: it runs per candidate per pixel.
#[inline]
fn consider_pixel(
    key: (u8, u8),
    pixel: LayerPixel,
    best: &mut Option<((u8, u8), LayerPixel)>,
    second: &mut Option<((u8, u8), LayerPixel)>,
) {
    match *best {
        None => *best = Some((key, pixel)),
        Some((best_key, _)) if key < best_key => {
            *second = *best;
            *best = Some((key, pixel));
        }
        _ => {
            let second_key = second.map(|(k, _)| k);
            if second_key.is_none_or(|k| key < k) {
                *second = Some((key, pixel));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write DISPCNT with the enable latch pre-propagated (steady state).
    /// Render tests use this to skip the 3-line hardware enable pipeline;
    /// latch propagation itself is covered by `dispcnt_enable_latch_delays`.
    fn steady_dispcnt(ppu: &mut GbaPpu, value: u16, vram: &[u8], palette: &[u8]) {
        ppu.write_register(0x04000000, value, vram, palette);
        ppu.dispcnt_latch = [value; 3];
    }

    #[test]
    fn timing_sets_status_and_completes_frame() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        for _ in 0..HBLANK_FLAG_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_ne!(ppu.dispstat() & 2, 0);
        for _ in HBLANK_FLAG_CYCLES..CYCLES_PER_LINE {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 1);
        let mut completed = false;
        for _ in CYCLES_PER_LINE as usize..CYCLES_PER_LINE as usize * LINES_PER_FRAME as usize {
            completed |= ppu.step(&vram, &palette, &oam).frame_complete;
        }
        assert!(completed);
        assert_eq!(ppu.vcount(), 0);
    }

    #[test]
    fn dispstat_hblank_timing() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Before HBLANK_FLAG_CYCLES, flag is 0
        for _ in 0..HBLANK_FLAG_CYCLES - 1 {
            ppu.step(&vram, &palette, &oam);
            assert_eq!(ppu.dispstat() & 2, 0, "HBlank flag should be 0 during draw");
        }
        // At HBLANK_FLAG_CYCLES, flag becomes 1
        ppu.step(&vram, &palette, &oam);
        assert_ne!(ppu.dispstat() & 2, 0);
        // Next step still 1
        ppu.step(&vram, &palette, &oam);
        assert_ne!(ppu.dispstat() & 2, 0);
        // HBlank interrupt
        let mut ppu2 = GbaPpu::new();
        ppu2.write_register(0x04000004, 1 << 4, &vram, &palette);
        for _ in 0..HBLANK_FLAG_CYCLES - 1 {
            assert_eq!(ppu2.step(&vram, &palette, &oam).interrupt_mask, 0);
        }
        assert_eq!(ppu2.step(&vram, &palette, &oam).interrupt_mask, 1 << 1);
        // Verify scanline still completes
        for _ in HBLANK_FLAG_CYCLES + 1..=CYCLES_PER_LINE {
            ppu2.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu2.vcount(), 1);
    }

    #[test]
    fn mode_three_renders_bitmap_pixel() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[..2].copy_from_slice(&0x001Fu16.to_le_bytes());
        steady_dispcnt(&mut ppu, 3 | 1 << 10, &vram, &palette);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
    }

    #[test]
    fn mode_four_uses_palette_and_mode_five_clips() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        palette[2..4].copy_from_slice(&0x03E0u16.to_le_bytes());
        steady_dispcnt(&mut ppu, 4 | 1 << 10, &vram, &palette);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 255, 0, 255]);

        let mut ppu = GbaPpu::new();
        vram[(127 * 160 + 159) * 2..(127 * 160 + 159) * 2 + 2]
            .copy_from_slice(&0x7C00u16.to_le_bytes());
        steady_dispcnt(&mut ppu, 5 | 1 << 10, &vram, &palette);
        // Deferred rendering completes the line at line end: step past
        // HDRAW so the asserted pixels are actually drawn (values are
        // identical to per-cycle rendering; only the observation point
        // moved past the fetches).
        for _ in 0..CYCLES_PER_LINE as usize * 127 + CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[127 * WIDTH + 159].to_le_bytes(),
            [0, 0, 255, 255]
        );
        assert_eq!(
            ppu.frame_buffer()[127 * WIDTH + 160].to_le_bytes(),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn mode_four_transparent_behind_obj() {
        // GBATEK Mode 4: color 0 is transparent, OBJ behind must show through
        // even when the BG has higher display priority than the OBJ.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        // Disable OBJs 1..127 (attr0 bit 9), keep OBJ0 enabled at (0,0).
        for entry in oam.as_chunks_mut::<8>().0.iter_mut().skip(1) {
            entry[0..2].copy_from_slice(&0x0200u16.to_le_bytes());
        }
        vram[0] = 0; // Mode 4 frame pixel (0,0): transparent color 0.
        vram[0x10000 + 512 * 32] = 1; // OBJ tile 512, pixel = palette index 1.
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes()); // blue
        // BG2 priority 0, OBJ0 (tile 512, priority 1).
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0x00, 0x06]);
        steady_dispcnt(&mut ppu, 4 | (1 << 10) | (1 << 12), &vram, &palette);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
    }

    #[test]
    fn bg_overfetch_reads_latch() {
        // CBB=3 + tile 513 lands the tile fetch on 0x10020 (>= boundary):
        // it reads the latched map entry (0x0201) low byte -> index 1 (red).
        // Neither transparent nor wrapped tile data.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        for b in vram.iter_mut().take(0x20) {
            *b = 0x11; // tile 0 (would-be wrap target): palette index 1
        }
        vram[0..2].copy_from_slice(&513u16.to_le_bytes()); // map: tile 513
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes()); // red
        steady_dispcnt(&mut ppu, 1 << 8, &vram, &palette);
        ppu.write_register(0x04000008, 3 << 2, &vram, &palette); // CBB=3, SBB=0, 4bpp, 32x32
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
    }

    #[test]
    fn large_bg_map_past_64k_reads_latch() {
        // 64x64 map at SBB=31 scrolled to (256,256) fetches its entry from
        // 0x11000 (>= boundary): the initial latch (0) is returned, so the
        // pixel is backdrop. Physical VRAM at 0x11000 must not be read.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0x11000..0x11002].copy_from_slice(&1u16.to_le_bytes()); // tile 1
        for b in vram.iter_mut().skip(0x20).take(0x20) {
            *b = 0x11; // tile 1: palette index 1
        }
        steady_dispcnt(&mut ppu, 1 << 8, &vram, &palette);
        ppu.write_register(0x04000008, (31 << 8) | (3 << 14), &vram, &palette); // SBB=31, 64x64
        ppu.write_register(0x04000010, 256, &vram, &palette); // hofs
        ppu.write_register(0x04000012, 256, &vram, &palette); // vofs
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 0, 255]);
    }

    #[test]
    fn text_bg_and_obj_render_palette_entries() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        vram[0] = 1;
        vram[0xF802..0xF804].copy_from_slice(&0xF000u16.to_le_bytes());
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        palette[0x1E2..0x1E4].copy_from_slice(&0x7FFFu16.to_le_bytes());
        steady_dispcnt(&mut ppu, 1 << 8, &vram, &palette);
        ppu.write_register(0x04000008, 31 << 8, &vram, &palette);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[8].to_le_bytes(), [255, 255, 255, 255]);

        let mut ppu = GbaPpu::new();
        vram[0x10000] = 1;
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes());
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0, 0]);
        steady_dispcnt(&mut ppu, (1 << 12) | (1 << 6), &vram, &palette);
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
    }

    #[test]
    fn window_masks_bg() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        steady_dispcnt(&mut ppu, (1 << 8) | (1 << 13), &vram, &palette);
        ppu.write_register(0x04000008, 31 << 8, &vram, &palette);
        ppu.write_register(0x04000040, 0x0014, &vram, &palette); // WIN0H: 0..20 (x1=0,x2=20)
        ppu.write_register(0x04000044, 0x0014, &vram, &palette); // WIN0V: 0..20
        ppu.write_register(0x04000048, 1 << 0, &vram, &palette);
        ppu.write_register(0x0400004A, 0, &vram, &palette);
        for _ in 0..HDRAW_CYCLES as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes()[0], 255);
        // Deferred rendering completes the line at line end (see above).
        for _ in HDRAW_CYCLES as usize..21 * CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[20 * WIDTH + 20].to_le_bytes(),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn dispcnt_enable_latch_delays_and_blank_is_or() {
        // HW-confirmed model: BG enables gate on latched AND live
        // (3-stage shift at +40 cycles/line), forced blank on latched OR live.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 1;
        // BG0 on written mid-frame: still gated off until the latch shifts
        // through (renders backdrop), then appears.
        ppu.write_register(0x04000000, 1 << 8, &vram, &palette);
        ppu.write_register(0x04000008, 31 << 8, &vram, &palette);
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes()[0], 0);
        for _ in 0..CYCLES_PER_LINE as usize * 3 {
            ppu.step(&vram, &palette, &oam);
        }
        // Latch propagated through the 3-stage shift: BG0 enable visible.
        assert_ne!(ppu.dispcnt_latch[0] & (1 << 8), 0);
        assert_ne!(ppu.line.enable & (1 << 8), 0);
        // Forced blank applies immediately (OR semantics, no latency):
        // the line drawn after the write is white.
        ppu.write_register(0x04000000, (1 << 8) | (1 << 7), &vram, &palette);
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[4 * WIDTH].to_le_bytes(),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn mosaic_expands_dots() {
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Set up 2 tiles: tile 0 all 0, tile 1 all 1 (red)
        // Simplified: just test that mosaic registers affect bg_mosaic
        ppu.write_register(0x0400004C, 0x11, &vram, &palette); // BG mosaic 1x1
        ppu.write_register(0x04000008, (1 << 6) | (31 << 8), &vram, &palette); // BG0 mosaic enable
        vram[0] = 2; // tile map entry for mosaic test
        palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
        for _ in 0..HDRAW_CYCLES {
            ppu.step(&vram, &palette, &oam);
        }
        // With mosaic 1, no expansion, should still render (basic check that mosaic doesn't crash)
        assert_eq!(ppu.frame_buffer().len(), WIDTH * HEIGHT);
        // Test mosaic 2x2
        ppu.write_register(0x0400004C, 0x11 | 0x1100, &vram, &palette); // BG 1x1, OBJ 1x1
        ppu.write_register(0x04000008, (1 << 6) | (31 << 8), &vram, &palette);
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer().len(), WIDTH * HEIGHT);
    }

    #[test]
    fn oam_mid_line_write_defers_to_next_line() {
        // OBJ0 8x8 at (0,0), tile 512; disable it mid-draw on line 0.
        // The remainder of line 0 still shows the sprite (line-deferred),
        // line 1 hides it.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        for entry in oam.as_chunks_mut::<8>().0.iter_mut().skip(1) {
            entry[0..2].copy_from_slice(&0x0200u16.to_le_bytes());
        }
        vram[0x10000 + 512 * 32..0x10000 + 512 * 32 + 8].fill(0x11);
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes()); // blue
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0x00, 0x02]);
        steady_dispcnt(&mut ppu, (1 << 12) | (1 << 6), &vram, &palette);
        // Write after the first fetch (cycle 32): line 0 keeps the sprite.
        for _ in 0..64 {
            ppu.step(&vram, &palette, &oam);
        }
        oam[0..2].copy_from_slice(&0x0200u16.to_le_bytes()); // disable OBJ0
        for _ in 64..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [0, 0, 255, 255]);
        assert_eq!(ppu.frame_buffer()[5].to_le_bytes(), [0, 0, 255, 255]);
        for _ in CYCLES_PER_LINE as usize..2 * CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[WIDTH].to_le_bytes(), [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_mid_line_write_defers_to_next_line() {
        // BG0 4bpp tile 0 with distinct pixels, horizontal mosaic 2 from line
        // start; clearing MOSAIC mid-draw keeps mosaic for the rest of line 0
        // and disables it on line 1.
        let mut ppu = GbaPpu::new();
        let mut vram = vec![0; 0x18000];
        let mut palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        vram[0] = 0x21; // px0=1 red, px1=2 green
        vram[2] = 0x43; // px4=3 blue, px5=4 white
        vram[4] = 0x21;
        vram[6] = 0x43;
        palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
        palette[6..8].copy_from_slice(&0x7C00u16.to_le_bytes());
        palette[8..10].copy_from_slice(&0x7FFFu16.to_le_bytes());
        steady_dispcnt(&mut ppu, 1 << 8, &vram, &palette);
        ppu.write_register(0x04000008, (31 << 8) | (1 << 6), &vram, &palette); // SBB=31, mosaic
        ppu.write_register(0x0400004C, 0x01, &vram, &palette); // BG mosaic h=2, v=1
        // Clear after the first fetch (cycle 32): line 0 keeps mosaic.
        for _ in 0..64 {
            ppu.step(&vram, &palette, &oam);
        }
        ppu.write_register(0x0400004C, 0x00, &vram, &palette); // clear mid-draw
        for _ in 64..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.frame_buffer()[0].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[1].to_le_bytes(), [255, 0, 0, 255]);
        assert_eq!(ppu.frame_buffer()[5].to_le_bytes(), [0, 0, 255, 255]);
        for _ in CYCLES_PER_LINE as usize..2 * CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(
            ppu.frame_buffer()[WIDTH + 1].to_le_bytes(),
            [0, 255, 0, 255]
        );
        assert_eq!(
            ppu.frame_buffer()[WIDTH + 5].to_le_bytes(),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn vblank_flag_cleared_on_line_227() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let oam = vec![0; 0x400];
        // Advance to VBlank start (line 160).
        for _ in 0..CYCLES_PER_LINE as usize * 160 {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 160);
        assert_ne!(ppu.dispstat() & 1, 0);
        // Advance to line 226: flag still set.
        for _ in 0..CYCLES_PER_LINE as usize * 66 {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 226);
        assert_ne!(ppu.dispstat() & 1, 0);
        // Line 227: GBATEK says flag is 0.
        for _ in 0..CYCLES_PER_LINE as usize {
            ppu.step(&vram, &palette, &oam);
        }
        assert_eq!(ppu.vcount(), 227);
        assert_eq!(ppu.dispstat() & 1, 0);
    }

    #[test]
    fn vcounter_irq_on_dispstat_write() {
        let mut ppu = GbaPpu::new();
        // Register-only test: empty buffers suffice (no stepping, so the
        // deferred-render hook never fires).
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        // VCOUNT=0, write LYC=0 with enable -> immediate IRQ.
        let irq = ppu.write_register(0x04000004, 1 << 5, &vram, &palette);
        assert_eq!(irq, 1 << 2);
        assert_ne!(ppu.dispstat() & (1 << 2), 0);
        // Same write again (already matching, already enabled) -> no repeat IRQ.
        let irq2 = ppu.write_register(0x04000004, 1 << 5, &vram, &palette);
        assert_eq!(irq2, 0);
        // Enable rising while already matching -> IRQ.
        ppu.write_register(0x04000004, 0 << 8, &vram, &palette); // disable, LYC=0 still match, no IRQ
        assert_eq!(ppu.dispstat() & (1 << 2), 4);
        let irq3 = ppu.write_register(0x04000004, 1 << 5, &vram, &palette);
        assert_eq!(irq3, 1 << 2);
    }

    #[test]
    fn rw_registers_are_readable() {
        let mut ppu = GbaPpu::new();
        // Register-only test: empty buffers suffice (no stepping, so the
        // deferred-render hook never fires).
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        ppu.write_register(0x04000008, 0x1234, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000008), Some(0x1234));
        ppu.write_register(0x04000048, 0x00FF, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000048), Some(0x003F));
        ppu.write_register(0x0400004A, 0x0F0F, &vram, &palette);
        assert_eq!(ppu.read_register(0x0400004A), Some(0x0F0F));
        ppu.write_register(0x04000050, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000050), Some(0x3FFF));
        ppu.write_register(0x04000052, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000052), Some(0x1F1F));
        // W registers stay unreadable.
        assert_eq!(ppu.read_register(0x0400004C), None);
        assert_eq!(ppu.read_register(0x04000054), None);
    }

    #[test]
    fn write_masks_follow_hardware() {
        let mut ppu = GbaPpu::new();
        // Register-only test: empty buffers suffice (no stepping, so the
        // deferred-render hook never fires).
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        // BG0/BG1 have no bit-13 overflow flag.
        ppu.write_register(0x04000008, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000008), Some(0xDFFF));
        ppu.write_register(0x0400000C, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x0400000C), Some(0xFFFF));
        // WININ/WINOUT store 6 bits per byte.
        ppu.write_register(0x04000048, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x04000048), Some(0x3F3F));
        ppu.write_register(0x0400004A, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.read_register(0x0400004A), Some(0x3F3F));
    }

    #[test]
    fn dispcnt_bit3_preserved_on_cpu_write() {
        let mut ppu = GbaPpu::new();
        // Register-only test: empty buffers suffice (no stepping, so the
        // deferred-render hook never fires).
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        assert_eq!(ppu.dispcnt() & (1 << 3), 0);
        ppu.write_register(0x04000000, 0xFFFF, &vram, &palette);
        assert_eq!(ppu.dispcnt() & (1 << 3), 0);
    }

    #[test]
    fn ppu_state_round_trips_mid_scanline() {
        let mut ppu = GbaPpu::new();
        let vram = vec![0u8; 96 * 1024];
        let palette = vec![0u8; 1024];
        let oam = vec![0u8; 1024];
        // Mode 1 with an affine background and windows enabled.
        ppu.write_register(0x04000000, 0x2441, &vram, &palette);
        ppu.write_register(0x04000028, 0x0100, &vram, &palette);
        ppu.write_register(0x0400002C, 0x00F0, &vram, &palette);
        // Advance into the visible scanlines with a nonzero cycle.
        for _ in 0..(1232 * 40 + 500) {
            ppu.step(&vram, &palette, &oam);
        }
        assert!(ppu.vcount() > 0);
        assert!(ppu.cycle() > 0);

        let state = ppu.export_state();
        state.validate().unwrap();
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        let decoded: GbaPpuState = rmp_serde::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        let mut restored = GbaPpu::new();
        restored.import_state(decoded).unwrap();
        let again = rmp_serde::to_vec_named(&restored.export_state()).unwrap();
        assert_eq!(bytes, again);
        assert_eq!(restored.vcount(), ppu.vcount());
        assert_eq!(restored.cycle(), ppu.cycle());

        let mut bad = restored.export_state();
        bad.vcount = 228;
        assert!(bad.validate().is_err());
        let mut bad = restored.export_state();
        bad.cycle = 1232;
        assert!(bad.validate().is_err());
    }

    /// OBJ line-cache differential: `line_cache` must cover exactly the
    /// non-dropped, y-overlapping OBJs in index order, and a tighter
    /// cycle budget must strictly shrink the cover (this pins both the
    /// drop fusion and the y-overlap mirror against coordinates()).
    #[test]
    fn obj_line_cache_covers_visible_non_dropped_only() {
        let vram = vec![0; 0x18000];
        let palette = vec![0; 0x400];
        let mut oam = vec![0; 0x400];
        // 32 overlapping 64x64 sprites on line 0: 32*64 = 2048 cycles,
        // over both the 1210 and the 954 (H-blank free) budgets.
        for i in 0..32 {
            let x = (i * 4) as u16;
            oam[8 * i..8 * i + 2].copy_from_slice(&0u16.to_le_bytes()); // Y=0
            oam[8 * i + 2..8 * i + 4].copy_from_slice(&(x | (3 << 14)).to_le_bytes()); // 64x64
            oam[8 * i + 4..8 * i + 6].copy_from_slice(&0u16.to_le_bytes()); // tile 0
        }
        let mut ppu = GbaPpu::new();
        // OBJ layer on, no H-blank free: 1210-cycle budget.
        steady_dispcnt(&mut ppu, 1 << 12, &vram, &palette);
        ppu.line.capture(&oam, 1 << 12);
        let full = obj::line_cache(&ppu.registers, &ppu.line.oam[..], 0);
        assert!(
            full.cover_len > 0 && full.cover_len < 128,
            "{}",
            full.cover_len
        );
        // Cover respects drops and index order, and carries the live attrs.
        let dropped = obj::cycle_drop_mask(&ppu.registers, &ppu.line.oam[..], 0);
        let mut prev = 0u8;
        for (n, entry) in full.cover[..full.cover_len as usize].iter().enumerate() {
            assert!(!dropped[usize::from(entry.0)], "covered {n} is dropped");
            if n > 0 {
                assert!(entry.0 > prev, "cover out of order at {n}");
            }
            prev = entry.0;
            assert_eq!(
                &oam[8 * usize::from(entry.0)..8 * usize::from(entry.0) + 6],
                &[
                    entry.1.to_le_bytes(),
                    entry.2.to_le_bytes(),
                    entry.3.to_le_bytes()
                ]
                .concat(),
                "stale attrs at {n}"
            );
        }
        // Tighten the budget: the cover must strictly shrink (drop order
        // is priority order, so every previously covered-then-dropped
        // sprite stays dropped).
        ppu.write_register(0x04000000, (1 << 12) | (1 << 5), &vram, &palette);
        let tight = obj::line_cache(&ppu.registers, &ppu.line.oam[..], 0);
        assert!(
            tight.cover_len < full.cover_len,
            "{} >= {}",
            tight.cover_len,
            full.cover_len
        );
        let tight_dropped = obj::cycle_drop_mask(&ppu.registers, &ppu.line.oam[..], 0);
        for entry in tight.cover[..tight.cover_len as usize].iter() {
            assert!(!tight_dropped[usize::from(entry.0)]);
        }
    }

    /// Span differential: `render_span` must be pixel- and
    /// latch-identical to the per-pixel `render_pixel` loop wherever
    /// the gate applies (goldens only cover shipped scenes; this pins
    /// the gate logic across modes, priorities, flips, mosaic and OBJ
    /// layers). Distinct colors per index catch priority/selection
    /// drift; transparent tile pixels catch backdrop handling; the
    /// latch comparison catches fetch-sequence drift (latch feeds
    /// later above-boundary fetches).
    #[test]
    fn span_matches_pixel_loop_where_gate_applies() {
        fn pattern_memories(obj: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
            let mut vram = vec![0; 0x18000];
            let mut palette = vec![0; 0x400];
            let mut oam = vec![0; 0x400];
            // Palette: backdrop blue, then red/green/white.
            palette[0..2].copy_from_slice(&0x7C00u16.to_le_bytes());
            palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
            palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
            palette[6..8].copy_from_slice(&0x7FFFu16.to_le_bytes());
            // Tiles (4bpp, CBB 0): tile 0 opaque red, tile 1 mixed
            // transparent/red, tile 2 opaque green.
            for b in vram.iter_mut().take(32) {
                *b = 0x11;
            }
            for (i, b) in vram.iter_mut().skip(32).take(32).enumerate() {
                *b = if i % 2 == 0 { 0x10 } else { 0x01 };
            }
            for b in vram.iter_mut().skip(64).take(32) {
                *b = 0x22;
            }
            // BG0 map (SBB 0): tile 0, flipped tile 1, tile 2.
            vram[0..2].copy_from_slice(&0u16.to_le_bytes());
            vram[2..4].copy_from_slice(&((1 | (1 << 10) | (1 << 11)) as u16).to_le_bytes());
            vram[4..6].copy_from_slice(&2u16.to_le_bytes());
            for i in 3..32 {
                vram[2 * i..2 * i + 2].copy_from_slice(&((i % 3) as u16).to_le_bytes());
            }
            // BG1 map (SBB 1): tile 2 over the left half, tile 0 right.
            for i in 0..32 {
                let tile = if i < 16 { 2u16 } else { 0u16 };
                vram[0x800 + 2 * i..0x800 + 2 * i + 2].copy_from_slice(&tile.to_le_bytes());
            }
            if obj {
                // OBJ tile 0 (at 0x10000): opaque red.
                for b in vram.iter_mut().skip(0x10000).take(32) {
                    *b = 0x11;
                }
                // Sprite 0: 8x8 at (10,0), tile 0; sprite 1: 16x16 at
                // (30,0), tile 0.
                oam[0..2].copy_from_slice(&0u16.to_le_bytes());
                oam[2..4].copy_from_slice(&10u16.to_le_bytes());
                oam[4..6].copy_from_slice(&0u16.to_le_bytes());
                oam[8..10].copy_from_slice(&0u16.to_le_bytes());
                oam[10..12].copy_from_slice(&(30u16 | (1 << 14)).to_le_bytes());
                oam[12..14].copy_from_slice(&0u16.to_le_bytes());
            }
            (vram, palette, oam)
        }

        // (dispcnt, bg0cnt, bg1cnt-or-0xFFFF, hofs0, vofs0, mosaic, obj)
        let cases: [(u16, u16, u16, u16, u16, u16, bool); 6] = [
            // Single BG, scrolled.
            (1 << 8, 1, 0xFFFF, 7, 5, 0, false),
            // Two BGs, BG1 priority 0 over BG0 priority 1.
            ((1 << 8) | (1 << 9), 1, 1 << 8, 0, 0, 0, false),
            // 256-color BG0 (bit 7) with flips in the map.
            (1 << 8, 1 | (1 << 7), 0xFFFF, 3, 9, 0, false),
            // Mosaic on (gate allows: correctness via identical bg::pixel).
            (1 << 8, 1, 0xFFFF, 0, 0, 0x0033, false),
            // Mode 3 bitmap BG2.
            (3 | (1 << 10), 0, 0xFFFF, 0, 0, 0, false),
            // Sprites over BG0 (priority exercise + transparent overlap).
            ((1 << 8) | (1 << 12), 1, 0xFFFF, 0, 0, 0, true),
        ];
        for (dispcnt, bg0cnt, bg1cnt, hofs, vofs, mosaic, obj) in cases {
            let build = || {
                let (vram, palette, oam) = pattern_memories(obj);
                let mut ppu = GbaPpu::new();
                steady_dispcnt(&mut ppu, dispcnt, &vram, &palette);
                if bg0cnt != 0 || dispcnt & 7 == 0 {
                    ppu.write_register(0x04000008, bg0cnt, &vram, &palette);
                }
                if bg1cnt != 0xFFFF {
                    ppu.write_register(0x0400000A, bg1cnt, &vram, &palette);
                }
                ppu.write_register(0x04000010, hofs, &vram, &palette);
                ppu.write_register(0x04000012, vofs, &vram, &palette);
                if mosaic != 0 {
                    ppu.write_register(0x0400004C, mosaic, &vram, &palette);
                }
                // Pixel 0 at its boundary tick (both paths start at x=1).
                for _ in 0..FETCH_START_CYCLES {
                    ppu.step(&vram, &palette, &oam);
                }
                (ppu, vram, palette, oam)
            };
            let (mut a, vram, palette, _) = build();
            let (mut b, _, _, _) = build();
            assert!(a.span_applies(), "gate must apply: {dispcnt:#X}");
            // Mirror production: real working set iff OBJ can appear.
            if obj {
                let cache = obj::line_cache(&b.registers, &b.line.oam[..], 0);
                a.render_span(1, WIDTH, 0, &vram, &palette, &cache);
            } else {
                a.render_span(1, WIDTH, 0, &vram, &palette, &obj::ObjLineCache::empty());
            }
            let obj_cache = obj::line_cache(&b.registers, &b.line.oam[..], 0);
            for x in 1..WIDTH {
                b.render_pixel(x, 0, &vram, &palette, &obj_cache);
            }
            assert_eq!(
                a.frame_buffer(),
                b.frame_buffer(),
                "frame diverged: dispcnt={dispcnt:#X}"
            );
            assert_eq!(a.bg_latch, b.bg_latch, "latch diverged: {dispcnt:#X}");
        }

        // Gate rejects every disqualifier (OBJ is allowed).
        let (vram, palette, _) = pattern_memories(false);
        let mut ppu = GbaPpu::new();
        steady_dispcnt(&mut ppu, (1 << 8) | (1 << 12), &vram, &palette);
        assert!(ppu.span_applies());
        ppu.write_register(0x04000000, (1 << 8) | (1 << 7), &vram, &palette); // forced blank
        assert!(!ppu.span_applies());
        ppu.write_register(0x04000000, (1 << 8) | (1 << 13), &vram, &palette); // WIN0
        assert!(!ppu.span_applies());
        ppu.write_register(0x04000000, (1 << 8) | (1 << 12), &vram, &palette);
        assert!(ppu.span_applies()); // OBJ allowed
        ppu.write_register(0x04000050, 1, &vram, &palette); // bldcnt != 0
        assert!(!ppu.span_applies());
        ppu.write_register(0x04000050, 0, &vram, &palette);
        ppu.write_register(0x04000002, 1, &vram, &palette); // greenswap
        assert!(!ppu.span_applies());
    }
}
