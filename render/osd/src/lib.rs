//! CPU OSD rasterizer: notification rows to a cropped RGBA layer.
//! One implementation shared by every GPU backend; backends only blit.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use cosmic_text::{CacheKey, SwashContent};
use nerust_render_traits::renderer::{OsdAnchor, OsdLayer, SafeInsets, osd_font_px};
use std::{collections::HashMap, rc::Rc};

/// One raster pill row: tag plus body in a single color.
/// Tag is a static level marker: no per-frame allocation at the shell boundary.
#[derive(Debug, Clone)]
pub struct OsdRow {
    pub tag: &'static str,
    pub text: String,
    pub rgb: (u8, u8, u8),
}

/// Status readout: label plus right-aligned value in a fixed-width
/// pill so digits never shift.
#[derive(Debug, Clone)]
pub struct OsdStatus {
    pub label: String,
    pub value: String,
    pub rgb: (u8, u8, u8),
}

/// Raster input: transient rows plus optional status, with corners.
#[derive(Debug, Clone, Default)]
pub struct OsdContent {
    pub rows: Vec<OsdRow>,
    pub status: Option<OsdStatus>,
    pub position: OsdAnchor,
    pub status_position: OsdAnchor,
}

/// System font preference. Extra scan roots arrive via
/// [`OsdRaster::with_font_dirs`].
fn best_sans_name(db: &fontdb::Database) -> Option<String> {
    for name in [
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Arial",
        "Helvetica",
    ] {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            ..fontdb::Query::default()
        };
        if db.query(&query).is_some() {
            return Some(name.to_string());
        }
    }
    let mut names: Vec<String> = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(family, _)| family.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .find(|name| name.to_lowercase().contains("sans"))
}

/// Fixed layout constants at scale 1.0, shared with the old overlay
/// math. [`OsdRaster::ui_scale`] multiplies them for dense displays.
const MARGIN: u32 = 8;
const ROW_GAP: u32 = 4;
const PILL_PAD_X: u32 = 6;
const PILL_PAD_Y: u32 = 2;
const PILL_RADIUS: u32 = 6;
const PILL_BG: (u8, u8, u8, u8) = (0, 0, 0, 184);

/// One shaped glyph: raster cache key plus integer line offset.
/// Measure and draw share the span, so the box always fits.
#[derive(Clone, Copy)]
struct ShapedGlyph {
    key: CacheKey,
    x: i32,
    y: i32,
}

/// One shaped line: glyph positions plus total advance.
/// Shared by cache: identical lines shape once per size.
#[derive(Clone)]
struct ShapedSpan {
    glyphs: Vec<ShapedGlyph>,
    width: f32,
    ascent: f32,
    descent: f32,
}

/// Cache key for one shaped line: interned text plus pixel size.
/// `Rc<str>` lookups borrow as `str`, so hits never allocate.
type ShapeKey = (Rc<str>, u32);

/// Interned gap and empty spans: every pill shares one allocation.
/// String literals are `'static` constants, so no global map is needed.
const GAP_STR: &str = " ";
const EMPTY_STR: &str = "";

/// Bounded line-shape cache: identical OSD lines shape once per size.
/// LRU order evicts the colder half past the cap; OSD text has a tiny live set.
const SHAPE_CACHE_CAP: usize = 256;

/// Rasterizer backed by cosmic-text shaping. Pixels are device pixels.
pub struct OsdRaster {
    font_system: FontSystem,
    swash: SwashCache,
    sans_name: String,
    has_fonts: bool,
    ui_scale: f32,
    safe_insets: SafeInsets,
    shape_cache: HashMap<Rc<str>, HashMap<u32, Rc<ShapedSpan>>>,
    shape_lru: std::collections::VecDeque<ShapeKey>,
}

impl OsdRaster {
    pub fn new() -> Self {
        Self::with_font_dirs(&[])
    }

    /// Extra font scan roots; still one scan total.
    /// Platform font paths (e.g. Android's `/system/fonts`) are the caller's vocabulary.
    pub fn with_font_dirs(dirs: &[&str]) -> Self {
        let mut font_system = FontSystem::new();
        for dir in dirs {
            font_system.db_mut().load_fonts_dir(dir);
        }
        let db = font_system.db();
        let has_fonts = !db.is_empty();
        let sans_name = best_sans_name(db).unwrap_or_default();
        if sans_name.is_empty() {
            log::warn!("osd sans query found nothing");
        }
        Self {
            font_system,
            swash: SwashCache::new(),
            sans_name,
            has_fonts,
            ui_scale: 1.0,
            safe_insets: SafeInsets::default(),
            shape_cache: HashMap::new(),
            shape_lru: std::collections::VecDeque::new(),
        }
    }

    /// Display-density scale for fonts and pill geometry (1.0 keeps
    /// the desktop-verified size). Frontends on dense screens shrink.
    pub fn set_ui_scale(&mut self, scale: f32) {
        self.ui_scale = scale.clamp(0.25, 4.0);
    }

    /// Extra edge padding for rounded corners, notches, or gesture bars.
    /// Zero by default so desktop output is unchanged.
    pub fn set_safe_insets(&mut self, insets: SafeInsets) {
        self.safe_insets = insets;
    }

    fn scaled(&self, px: u32) -> u32 {
        (px as f32 * self.ui_scale + 0.5) as u32
    }

    pub fn has_fonts(&self) -> bool {
        self.has_fonts
    }

    /// Rasterize rows into a cropped layer. `None` when empty/fontless.
    pub fn rasterize(
        &mut self,
        content: &OsdContent,
        surface_w: u32,
        surface_h: u32,
    ) -> Option<OsdLayer> {
        if surface_w == 0 || surface_h == 0 || !self.has_fonts() {
            return None;
        }
        let font_px = osd_font_px(surface_h, self.ui_scale);
        let boxes = self.layout(content, font_px, surface_w, surface_h)?;
        let (min_x, min_y, max_x, max_y) =
            boxes
                .iter()
                .fold((u32::MAX, u32::MAX, 0, 0), |(x0, y0, x1, y1), b| {
                    (
                        x0.min(b.x),
                        y0.min(b.y),
                        x1.max(b.x + b.w),
                        y1.max(b.y + b.h),
                    )
                });
        let width = max_x - min_x;
        let height = max_y - min_y;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let pad_x = self.scaled(PILL_PAD_X);
        let gap = self.scaled(ROW_GAP);
        let radius = self.scaled(PILL_RADIUS);
        for b in &boxes {
            let base_y = b.y - min_y;
            match &b.body {
                RowBody::Line {
                    tag,
                    text,
                    gap,
                    ascent,
                    descent,
                } => {
                    draw_pill(&mut rgba, width, b.x - min_x, b.y - min_y, b.w, b.h, radius);
                    let baseline = self.centered_baseline(base_y, b.h, *ascent, *descent);
                    let mut pen = (b.x - min_x + pad_x) as i32;
                    self.draw_span(&mut rgba, width, tag, pen, baseline, b.rgb);
                    pen += tag.width.ceil() as i32 + *gap as i32;
                    self.draw_span(&mut rgba, width, text, pen, baseline, b.rgb);
                }
                RowBody::Pair {
                    outer,
                    inner,
                    ascent,
                    descent,
                    outer_left,
                } => {
                    // Two separate pills with a margin: value stays on
                    // the margin side so digits never shift.
                    let baseline = self.centered_baseline(base_y, b.h, *ascent, *descent);
                    let outer_w = outer.span.width.ceil() as u32 + pad_x * 2;
                    let inner_w = inner.span.width.ceil() as u32 + pad_x * 2;
                    let cbx = (b.x - min_x) as i32;
                    let (ox, ix) = if *outer_left {
                        (cbx, cbx + outer_w as i32 + gap as i32)
                    } else {
                        (cbx + b.w as i32 - outer_w as i32, cbx)
                    };
                    draw_pill(&mut rgba, width, ox as u32, base_y, outer_w, b.h, radius);
                    draw_pill(&mut rgba, width, ix as u32, base_y, inner_w, b.h, radius);
                    self.draw_span(
                        &mut rgba,
                        width,
                        &outer.span,
                        ox + pad_x as i32,
                        baseline,
                        b.rgb,
                    );
                    self.draw_span(
                        &mut rgba,
                        width,
                        &inner.span,
                        ix + pad_x as i32,
                        baseline,
                        b.rgb,
                    );
                }
            }
        }
        Some(OsdLayer::new(min_x, min_y, width, height, rgba))
    }

    fn layout(
        &mut self,
        content: &OsdContent,
        font_px: u32,
        surface_w: u32,
        surface_h: u32,
    ) -> Option<Vec<PlacedRow>> {
        let (top, bottom) = Self::collect_entries(content);
        if top.is_empty() && bottom.is_empty() {
            return None;
        }
        let mut boxes = Vec::new();
        self.place_entries(top, true, font_px, surface_w, surface_h, &mut boxes);
        self.place_entries(bottom, false, font_px, surface_w, surface_h, &mut boxes);
        Some(boxes)
    }

    /// Split rows and status by vertical edge; borrows content, never one string.
    fn collect_entries(content: &OsdContent) -> (Vec<Entry<'_>>, Vec<Entry<'_>>) {
        let same_corner = content.position == content.status_position;
        let transient_top = content.position.is_top();
        let status_top = content.status_position.is_top();
        let mut top: Vec<Entry<'_>> = Vec::new();
        let mut bottom: Vec<Entry<'_>> = Vec::new();
        for row in &content.rows {
            let entry = Entry::Line {
                tag: row.tag,
                text: row.text.as_str(),
                rgb: row.rgb,
                anchor: content.position,
            };
            if transient_top {
                top.push(entry);
            } else {
                bottom.push(entry);
            }
        }
        if let Some(status) = content.status.as_ref() {
            let entry = Entry::Status(status, content.status_position);
            if same_corner {
                if transient_top {
                    top.insert(0, entry);
                } else {
                    bottom.push(entry);
                }
            } else if status_top {
                top.insert(0, entry);
            } else {
                bottom.push(entry);
            }
        }
        (top, bottom)
    }

    /// Shape one edge: opposite sides sharing the edge sit side by side
    /// on one row instead of stacking.
    fn place_entries(
        &mut self,
        list: Vec<Entry>,
        from_top: bool,
        font_px: u32,
        surface_w: u32,
        surface_h: u32,
        boxes: &mut Vec<PlacedRow>,
    ) {
        let mut shaped: Vec<Shaped> = list
            .into_iter()
            .map(|entry| self.shape_entry(entry, font_px))
            .collect();
        if !from_top {
            shaped.reverse();
        }
        let margin = self.scaled(MARGIN);
        let mut cursor = if from_top {
            margin + self.safe_insets.top
        } else {
            surface_h
                .saturating_sub(margin)
                .saturating_sub(self.safe_insets.bottom)
        };
        while !shaped.is_empty() {
            let end = Self::pair_width(&shaped);
            let row_h = shaped[..end]
                .iter()
                .map(|entry| entry.row_h)
                .max()
                .unwrap_or(0);
            for entry in shaped.drain(..end) {
                boxes.push(PlacedRow {
                    x: self.anchor_x(entry.anchor, entry.w, surface_w),
                    y: Self::entry_y(from_top, cursor, entry.row_h),
                    w: entry.w,
                    h: entry.row_h,
                    rgb: entry.rgb,
                    body: entry.body,
                });
            }
            cursor = self.advance_cursor(from_top, cursor, row_h);
        }
    }

    /// Entries sharing a row: two when the leading pair anchors on
    /// opposite sides, one otherwise.
    fn pair_width(shaped: &[Shaped]) -> usize {
        if shaped.len() > 1 && shaped[0].anchor.is_left() != shaped[1].anchor.is_left() {
            2
        } else {
            1
        }
    }

    /// Horizontal anchor: left edge pins to the margin, right edge
    /// pins the box against the margin.
    fn anchor_x(&self, anchor: OsdAnchor, w: u32, surface_w: u32) -> u32 {
        let margin = self.scaled(MARGIN);
        if anchor.is_left() {
            margin + self.safe_insets.left
        } else {
            surface_w
                .saturating_sub(w + margin)
                .saturating_sub(self.safe_insets.right)
        }
    }

    /// Row origin: top rows grow down from the cursor, bottom rows
    /// align their bottoms to it.
    fn entry_y(from_top: bool, cursor: u32, row_h: u32) -> u32 {
        if from_top {
            cursor
        } else {
            cursor.saturating_sub(row_h)
        }
    }

    /// Cursor step past one placed row plus the inter-row margin.
    fn advance_cursor(&self, from_top: bool, cursor: u32, row_h: u32) -> u32 {
        let gap = self.scaled(ROW_GAP);
        if from_top {
            cursor + row_h + gap
        } else {
            cursor.saturating_sub(row_h).saturating_sub(gap)
        }
    }

    /// Shape one entry: intrinsic width plus row height.
    /// Tag and body are two cached spans; the pill covers both plus one space.
    fn shape_entry(&mut self, entry: Entry<'_>, font_px: u32) -> Shaped {
        let pad_x = self.scaled(PILL_PAD_X);
        match entry {
            Entry::Line {
                tag,
                text,
                rgb,
                anchor,
            } => {
                let tag_span = self.shape_cached(tag, font_px);
                let text_span = self.shape_cached(text, font_px);
                let gap = self.shape_cached_rc(GAP_STR, font_px).width;
                let ascent = tag_span.ascent.max(text_span.ascent);
                let descent = tag_span.descent.min(text_span.descent);
                let row_h = self.row_height(font_px, ascent, descent);
                let w = tag_span.width.ceil() as u32
                    + gap.ceil() as u32
                    + text_span.width.ceil() as u32
                    + pad_x * 2;
                Shaped {
                    anchor,
                    body: RowBody::Line {
                        tag: tag_span,
                        text: text_span,
                        gap: gap.ceil() as u32,
                        ascent,
                        descent,
                    },
                    rgb,
                    w,
                    row_h,
                }
            }
            Entry::Status(status, anchor) => self.shape_status(status, anchor, font_px),
        }
    }

    /// Shape the status readout: solo value pill, or a label/value pair.
    /// The value pill's outer edge pins to the margin either way.
    fn shape_status(&mut self, status: &OsdStatus, anchor: OsdAnchor, font_px: u32) -> Shaped {
        let label = self.shape_cached(status.label.as_str(), font_px);
        let value = self.shape_cached(status.value.as_str(), font_px);
        if status.label.is_empty() {
            // Solo value pill, as before.
            let row_h = self.row_height(font_px, value.ascent, value.descent);
            let pad_x = self.scaled(PILL_PAD_X);
            let w = value.width.ceil() as u32 + pad_x * 2;
            let gap = self.shape_cached_rc(GAP_STR, font_px).width;
            return Shaped {
                anchor,
                body: RowBody::Line {
                    tag: value.clone(),
                    text: self.shape_cached_rc(EMPTY_STR, font_px),
                    gap: gap.ceil() as u32,
                    ascent: value.ascent,
                    descent: value.descent,
                },
                rgb: status.rgb,
                w,
                row_h,
            };
        }
        let ascent = label.ascent.max(value.ascent);
        let descent = label.descent.min(value.descent);
        let row_h = self.row_height(font_px, ascent, descent);
        let outer = StatusSpan { span: value };
        let inner = StatusSpan { span: label };
        let pad_x = self.scaled(PILL_PAD_X);
        let gap = self.scaled(ROW_GAP);
        let outer_w = outer.span.width.ceil() as u32 + pad_x * 2;
        let inner_w = inner.span.width.ceil() as u32 + pad_x * 2;
        Shaped {
            anchor,
            body: RowBody::Pair {
                outer,
                inner,
                ascent,
                descent,
                outer_left: anchor.is_left(),
            },
            rgb: status.rgb,
            w: outer_w + gap + inner_w,
            row_h,
        }
    }

    /// Cached shape: identical lines shape once per pixel size.
    /// Borrowed lookup hits without allocating; misses intern one `Rc<str>`.
    fn shape_cached(&mut self, line: &str, font_px: u32) -> Rc<ShapedSpan> {
        if let Some(hit) = self
            .shape_cache
            .get(line)
            .and_then(|sizes| sizes.get(&font_px))
            .cloned()
        {
            self.touch_lru(line, font_px);
            return hit;
        }
        let key: Rc<str> = Rc::from(line);
        let span = Rc::new(self.shape_uncached(line, font_px));
        self.insert_lru(Rc::clone(&key), font_px, Rc::clone(&span));
        span
    }

    /// Interned-key variant: gap and empty spans share one allocation.
    fn shape_cached_rc(&mut self, key: &'static str, font_px: u32) -> Rc<ShapedSpan> {
        if let Some(hit) = self
            .shape_cache
            .get(key)
            .and_then(|sizes| sizes.get(&font_px))
            .cloned()
        {
            self.touch_lru(key, font_px);
            return hit;
        }
        let span = Rc::new(self.shape_uncached(key, font_px));
        self.insert_lru(Rc::from(key), font_px, Rc::clone(&span));
        span
    }

    /// Test seam for shaping assertions: production shapes through the cache.
    #[cfg(test)]
    fn shape_line(&mut self, line: &str, font_px: u32) -> Rc<ShapedSpan> {
        self.shape_cached(line, font_px)
    }

    /// Mark a hit most-recently-used; linear move, cap is tiny.
    fn touch_lru(&mut self, line: &str, font_px: u32) {
        if let Some(pos) = self
            .shape_lru
            .iter()
            .position(|(text, size)| &**text == line && *size == font_px)
        {
            let key = self.shape_lru.remove(pos).expect("lru position");
            self.shape_lru.push_back(key);
        }
    }

    /// Record a miss; evict the colder half past the cap.
    fn insert_lru(&mut self, key: Rc<str>, font_px: u32, span: Rc<ShapedSpan>) {
        self.shape_cache
            .entry(Rc::clone(&key))
            .or_default()
            .insert(font_px, span);
        self.shape_lru.push_back((key, font_px));
        while self.shape_lru.len() > SHAPE_CACHE_CAP {
            let drop = self.shape_lru.len() - SHAPE_CACHE_CAP + SHAPE_CACHE_CAP / 2;
            for _ in 0..drop {
                let Some((text, size)) = self.shape_lru.pop_front() else {
                    break;
                };
                let empty = self
                    .shape_cache
                    .get_mut(&text)
                    .map(|sizes| {
                        sizes.remove(&size);
                        sizes.is_empty()
                    })
                    .unwrap_or(true);
                if empty {
                    self.shape_cache.remove(&text);
                }
            }
        }
    }

    /// Live entry count across all sizes; eviction keeps this bounded.
    #[cfg(test)]
    fn shape_cache_len(&self) -> usize {
        self.shape_lru.len()
    }

    /// Uncached shaping behind [`OsdRaster::shape_cached`].
    /// One cosmic buffer per miss; hits never touch the font system.
    fn shape_uncached(&mut self, line: &str, font_px: u32) -> ShapedSpan {
        let size = font_px as f32;
        let metrics = Metrics::new(size, size);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(8192.0), None);
        let family = if self.sans_name.is_empty() {
            Family::SansSerif
        } else {
            Family::Name(&self.sans_name)
        };
        buffer.set_text(line, &Attrs::new().family(family), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let Some(run) = buffer.layout_runs().next() else {
            return ShapedSpan {
                glyphs: Vec::new(),
                width: 0.0,
                ascent: size,
                descent: 0.0,
            };
        };
        let ascent = run.line_y - run.line_top;
        let glyphs = run
            .glyphs
            .iter()
            .map(|glyph| {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                ShapedGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                }
            })
            .collect();
        ShapedSpan {
            glyphs,
            width: run.line_w,
            ascent,
            // Rest of the em below the baseline; the row formula
            // floors the pill height, so this only needs to be stable.
            descent: ascent - size,
        }
    }

    fn row_height(&self, font_px: u32, ascent: f32, descent: f32) -> u32 {
        let pad_y = self.scaled(PILL_PAD_Y);
        let content_h = (ascent - descent).ceil().max(1.0) as u32;
        (font_px * 16 / 10 + pad_y * 2).max(content_h + pad_y * 2)
    }

    /// Blit one shaped span at an integer base position.
    /// Glyph images come from the shared swash cache.
    fn draw_span(
        &mut self,
        rgba: &mut [u8],
        stride_w: u32,
        span: &ShapedSpan,
        x0: i32,
        baseline: i32,
        rgb: (u8, u8, u8),
    ) {
        for glyph in &span.glyphs {
            let Some(image) = self.swash.get_image(&mut self.font_system, glyph.key) else {
                continue;
            };
            let SwashContent::Mask = image.content else {
                continue;
            };
            let (w, h) = (image.placement.width, image.placement.height);
            if w == 0 || h == 0 {
                continue;
            }
            // Same mapping as cosmic Buffer::draw: integer pen plus
            // placement, with the y offset negated to screen coords.
            let base_x = x0 + glyph.x + image.placement.left;
            let base_y = baseline + glyph.y - image.placement.top;
            let row_bytes = w as usize;
            for (row, chunk) in image.data.chunks_exact(row_bytes).enumerate() {
                for (col, coverage) in chunk.iter().enumerate() {
                    if *coverage == 0 {
                        continue;
                    }
                    let (dx, dy) = (base_x + col as i32, base_y + row as i32);
                    if dx < 0 || dy < 0 {
                        continue;
                    }
                    let offset = (dy as u32 * stride_w + dx as u32) as usize * 4;
                    if let Some(pixel) = rgba.get_mut(offset..offset + 4) {
                        blend_over(pixel, rgb.0, rgb.1, rgb.2, *coverage);
                    }
                }
            }
        }
    }
}

impl Default for OsdRaster {
    fn default() -> Self {
        Self::new()
    }
}

struct PlacedRow {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    rgb: (u8, u8, u8),
    body: RowBody,
}

/// Layout entry: each carries its own anchor corner for split placement.
/// Borrows the content; tag and body shape as two cached spans, never one string.
enum Entry<'a> {
    Line {
        tag: &'static str,
        text: &'a str,
        rgb: (u8, u8, u8),
        anchor: OsdAnchor,
    },
    Status(&'a OsdStatus, OsdAnchor),
}

/// Shaped entry: intrinsic width plus row height.
struct Shaped {
    anchor: OsdAnchor,
    body: RowBody,
    rgb: (u8, u8, u8),
    w: u32,
    row_h: u32,
}

enum RowBody {
    Line {
        tag: Rc<ShapedSpan>,
        text: Rc<ShapedSpan>,
        gap: u32,
        ascent: f32,
        descent: f32,
    },
    Pair {
        outer: StatusSpan,
        inner: StatusSpan,
        ascent: f32,
        descent: f32,
        outer_left: bool,
    },
}

struct StatusSpan {
    span: Rc<ShapedSpan>,
}

/// Baseline that centers the content block in the pill. The pill is
/// taller than the text, so top-anchoring leaves the bottom empty.
impl OsdRaster {
    fn centered_baseline(&self, base_y: u32, row_h: u32, ascent: f32, descent: f32) -> i32 {
        let content_h = ascent - descent;
        let pad_y = self.scaled(PILL_PAD_Y) as f32;
        let extra = row_h as f32 - (content_h + pad_y * 2.0);
        base_y as i32 + pad_y as i32 + (extra / 2.0).round() as i32 + ascent as i32
    }
}

/// Integer straight-alpha blend of one pixel over the layer buffer.
/// Round-half-up fixed point; no FPU on the per-pixel path.
fn blend_over(dst: &mut [u8], r: u8, g: u8, b: u8, a: u8) {
    let alpha = u32::from(a);
    let inv = 255 - alpha;
    dst[0] = ((u32::from(r) * alpha + u32::from(dst[0]) * inv + 127) / 255) as u8;
    dst[1] = ((u32::from(g) * alpha + u32::from(dst[1]) * inv + 127) / 255) as u8;
    dst[2] = ((u32::from(b) * alpha + u32::from(dst[2]) * inv + 127) / 255) as u8;
    dst[3] = ((alpha * 255 + u32::from(dst[3]) * inv + 127) / 255).min(255) as u8;
}

/// Rounded-rect pill fill on the layer (top-left origin, cropped space).
fn draw_pill(rgba: &mut [u8], stride_w: u32, x: u32, y: u32, w: u32, h: u32, radius: u32) {
    let radius = radius.min(w / 2).min(h / 2) as f32;
    for row in 0..h {
        for col in 0..w {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let nearest_x = px.clamp(radius, w as f32 - radius);
            let nearest_y = py.clamp(radius, h as f32 - radius);
            if (px - nearest_x).hypot(py - nearest_y) <= radius {
                let offset = ((y + row) * stride_w + (x + col)) as usize * 4;
                if let Some(pixel) = rgba.get_mut(offset..offset + 4) {
                    blend_over(pixel, PILL_BG.0, PILL_BG.1, PILL_BG.2, PILL_BG.3);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{OsdContent, OsdRow, OsdStatus};
    use nerust_render_traits::renderer::SafeInsets;

    fn content() -> OsdContent {
        OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]",
                text: "Paused".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::TopLeft,
        }
    }

    #[test]
    fn empty_content_yields_none() {
        let mut raster = OsdRaster::new();
        assert!(raster.rasterize(&OsdContent::default(), 960, 720).is_none());
        assert!(raster.rasterize(&content(), 0, 720).is_none());
    }

    #[test]
    fn ascii_rows_produce_a_cropped_layer() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        let layer = raster.rasterize(&content(), 960, 720).expect("ascii rows");
        assert_eq!(
            layer.rgba.len(),
            layer.width as usize * layer.height as usize * 4
        );
        assert!(layer.width < 960 && layer.height < 720);
        // Pill corners stay transparent (radius), text is opaque somewhere.
        assert_eq!(layer.rgba[3], 0);
        assert!(
            layer
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 200)
        );
        // Deterministic: the glyph cache makes the second pass identical.
        let again = raster.rasterize(&content(), 960, 720).expect("cached");
        assert_eq!(layer.rgba, again.rgba);
    }

    #[test]
    fn safe_insets_shift_the_layer_inward() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        let plain = raster.rasterize(&content(), 960, 720).expect("plain");
        raster.set_safe_insets(SafeInsets {
            top: 60,
            right: 40,
            bottom: 60,
            left: 40,
        });
        let inset = raster.rasterize(&content(), 960, 720).expect("inset");
        // Bottom-left anchor: the box moves right and up.
        assert!(inset.x > plain.x);
        assert!(inset.y < plain.y);
        assert_eq!(inset.width, plain.width);
        assert_eq!(inset.height, plain.height);
    }

    #[test]
    fn row_height_follows_the_shared_formula() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // 720p: font 16px, row 16*16/10+4 = 29.
        let content = OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]",
                text: "Paused".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::TopLeft,
        };
        let layer = raster.rasterize(&content, 960, 720).expect("row");
        assert_eq!(layer.height, 16 * 16 / 10 + 4);
    }

    #[test]
    fn corners_mirror_position() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        let layer = raster.rasterize(&content(), 960, 720).expect("layer");
        assert!(layer.x <= 8);
        assert!(layer.y + layer.height + 8 >= 720);
        let mut top_right = content();
        top_right.position = OsdAnchor::TopRight;
        let upper = raster.rasterize(&top_right, 960, 720).expect("layer");
        assert!(upper.x + upper.width + 8 >= 960);
        assert!(upper.y <= 8);
    }

    #[test]
    fn advanced_shaping_kerns_pairs() {
        let mut raster = super::OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // Full GPOS shaping must tighten kerned pairs versus isolated advances.
        // Skip when the system font lacks this pair; font specifics are not asserted.
        let pair = raster.shape_line("AV", 16).width;
        let isolated = raster.shape_line("A", 16).width + raster.shape_line("V", 16).width;
        if pair >= isolated {
            eprintln!("SKIP: system font has no AV kern");
            return;
        }
        assert!(pair < isolated, "AV must kern: {pair} vs {isolated}");
    }

    #[test]
    fn cjk_renders_nonblank() {
        let mut raster = super::OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        let content = OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]",
                text: "一時停止しました".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::TopLeft,
        };
        let layer = raster.rasterize(&content, 960, 720).expect("cjk rows");
        let lit = layer
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 200)
            .count();
        assert!(lit > 50, "cjk text must produce opaque pixels, got {lit}");
        // Body-only: the tag alone must not satisfy the assert above.
        let body = OsdContent {
            rows: vec![OsdRow {
                tag: "",
                text: "一時停止しました".to_string(),
                rgb: (255, 255, 255),
            }],
            ..content
        };
        let layer = raster.rasterize(&body, 960, 720).expect("cjk body");
        let body_lit = layer
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 200)
            .count();
        assert!(body_lit > 50, "cjk body must render, got {body_lit}");
    }

    #[test]
    fn space_advances_the_pen() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        let plain_w = raster.shape_line("ab", 16).width;
        let spaced_w = raster.shape_line("a b", 16).width;
        assert!(
            spaced_w > plain_w + 1.0,
            "space must advance the pen: {plain_w} vs {spaced_w}"
        );
    }

    #[test]
    fn status_orders_value_outside_and_packs_tight() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // Value outside (margin side), label inside, tight pill.
        let content = |label: &str, value: &str, anchor: OsdAnchor| OsdContent {
            rows: Vec::new(),
            status: Some(OsdStatus {
                label: label.to_string(),
                value: value.to_string(),
                rgb: (200, 200, 200),
            }),
            position: OsdAnchor::BottomLeft,
            status_position: anchor,
        };
        let left = OsdAnchor::BottomLeft;
        let right = OsdAnchor::BottomRight;
        let label_w = raster.shape_line("Paused", 16).width;
        // Left anchor: value first, so the digit block starts at the pad.
        let running = raster
            .rasterize(&content("", "60.1 fps", left), 960, 720)
            .expect("status layer");
        let value_w = raster.shape_line("60.1 fps", 16).width;
        assert_eq!(running.width, value_w.ceil() as u32 + super::PILL_PAD_X * 2);
        // Paused adds a second pill plus the inter-pill margin,
        // symmetric on both anchors.
        let pair_w = |a: f32, b: f32| {
            a.ceil() as u32
                + super::PILL_PAD_X * 2
                + super::ROW_GAP
                + b.ceil() as u32
                + super::PILL_PAD_X * 2
        };
        let paused_left = raster
            .rasterize(&content("Paused", "60.1 fps", left), 960, 720)
            .expect("status layer");
        let paused_right = raster
            .rasterize(&content("Paused", "60.1 fps", right), 960, 720)
            .expect("status layer");
        let expect = pair_w(value_w, label_w);
        assert_eq!(paused_left.width, expect);
        assert_eq!(paused_right.width, expect);
        // The margin between the pills stays transparent (zero-alpha seam).
        // The value pill stays on the margin side either way.
        let seam_left = value_w.ceil() as u32 + super::PILL_PAD_X * 2 + super::ROW_GAP / 2;
        for y in 0..paused_left.height {
            let alpha = paused_left.rgba
                [(y as usize * paused_left.width as usize + seam_left as usize) * 4 + 3];
            assert_eq!(alpha, 0, "seam must be transparent at y={y}");
        }
        let seam_right = label_w.ceil() as u32 + super::PILL_PAD_X * 2 + super::ROW_GAP / 2;
        for y in 0..paused_right.height {
            let alpha = paused_right.rgba
                [(y as usize * paused_right.width as usize + seam_right as usize) * 4 + 3];
            assert_eq!(alpha, 0, "seam must be transparent at y={y}");
        }
    }

    #[test]
    fn status_right_pins_value_to_margin() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // Right anchors keep the value pill on the margin side, so the fps
        // digits hold still instead of sliding with the label.
        let content = |label: &str| OsdContent {
            rows: Vec::new(),
            status: Some(OsdStatus {
                label: label.to_string(),
                value: "60.1 fps".to_string(),
                rgb: (200, 200, 200),
            }),
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::BottomRight,
        };
        let running = raster.rasterize(&content(""), 960, 720).expect("layer");
        let paused = raster
            .rasterize(&content("Paused"), 960, 720)
            .expect("layer");
        assert_eq!(running.x + running.width + super::MARGIN, 960);
        assert_eq!(paused.x + paused.width + super::MARGIN, 960);
    }

    #[test]
    fn opposite_sides_share_one_row() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // Notification bottom-left plus status bottom-right: one row,
        // spanning margin to margin instead of stacking.
        let content = OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]",
                text: "Paused".to_string(),
                rgb: (255, 255, 255),
            }],
            status: Some(OsdStatus {
                label: String::new(),
                value: "60.1 fps".to_string(),
                rgb: (200, 200, 200),
            }),
            position: OsdAnchor::BottomLeft,
            status_position: OsdAnchor::BottomRight,
        };
        let layer = raster.rasterize(&content, 960, 720).expect("layer");
        assert_eq!(layer.height, 16 * 16 / 10 + 4);
        assert!(layer.x <= super::MARGIN);
        assert_eq!(layer.x + layer.width + super::MARGIN, 960);
    }

    #[test]
    fn blend_over_matches_straight_alpha_math() {
        let mut pixel = [0u8, 0, 0, 0];
        blend_over(&mut pixel, 255, 255, 255, 255);
        assert_eq!(pixel, [255, 255, 255, 255]);
        let mut pixel = [0u8, 0, 0, 0];
        blend_over(&mut pixel, 0, 0, 0, 184);
        assert_eq!(pixel[3], 184);
        blend_over(&mut pixel, 0, 0, 0, 184);
        assert!(pixel[3] > 200);
    }

    #[test]
    fn shape_hits_share_one_span_without_realloc() {
        let mut raster = OsdRaster::new();
        let first = raster.shape_cached("Paused", 16);
        let second = raster.shape_cached("Paused", 16);
        // Borrowed lookup: same `Rc` back, no reshape and no new span.
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(raster.shape_cache_len(), 1);
        // Different size is a distinct entry under the same interned text.
        let big = raster.shape_cached("Paused", 32);
        assert!(!Rc::ptr_eq(&first, &big));
        assert_eq!(raster.shape_cache_len(), 2);
    }

    #[test]
    fn gap_and_empty_share_interned_spans() {
        let mut raster = OsdRaster::new();
        let gap = raster.shape_cached(" ", 16);
        let gap_again = raster.shape_cached_rc(GAP_STR, 16);
        assert!(Rc::ptr_eq(&gap, &gap_again));
        let empty = raster.shape_cached("", 16);
        let empty_again = raster.shape_cached_rc(EMPTY_STR, 16);
        assert!(Rc::ptr_eq(&empty, &empty_again));
        assert!(!Rc::ptr_eq(&gap, &empty));
    }

    #[test]
    fn overflow_evicts_cold_half_and_keeps_hot() {
        let mut raster = OsdRaster::new();
        let hot = raster.shape_cached("hot", 16);
        for i in 0..(super::SHAPE_CACHE_CAP + 64) {
            raster.shape_cached(&format!("line{i:04}"), 16);
        }
        // Bounded: never the full insert count, never a wholesale clear.
        assert!(raster.shape_cache_len() <= super::SHAPE_CACHE_CAP);
        assert!(raster.shape_cache_len() > super::SHAPE_CACHE_CAP / 2);
        // Untouched oldest entries evicted; re-touch keeps a survivor hot.
        let hot_again = raster.shape_cached("hot", 16);
        assert!(!Rc::ptr_eq(&hot, &hot_again), "cold head must evict");
        let warm = raster.shape_cached("warm", 16);
        for i in 0..64 {
            raster.shape_cached(&format!("fresh{i:04}"), 16);
            raster.shape_cached("warm", 16);
        }
        assert!(Rc::ptr_eq(&warm, &raster.shape_cached("warm", 16)));
    }
}
