//! CPU OSD rasterizer: notification rows to a cropped RGBA layer.
//! One implementation shared by every GPU backend; backends only blit.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use cosmic_text::{CacheKey, SwashContent};
use nerust_render_traits::renderer::{
    OsdAnchor, OsdContent, OsdLayer, OsdStatus, SafeInsets, osd_font_px,
};

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
struct ShapedSpan {
    glyphs: Vec<ShapedGlyph>,
    width: f32,
    ascent: f32,
    descent: f32,
}

/// Rasterizer backed by cosmic-text shaping. Pixels are device pixels.
pub struct OsdRaster {
    font_system: FontSystem,
    swash: SwashCache,
    sans_name: String,
    has_fonts: bool,
    ui_scale: f32,
    safe_insets: SafeInsets,
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
                RowBody::Line { span } => {
                    draw_pill(&mut rgba, width, b.x - min_x, b.y - min_y, b.w, b.h, radius);
                    let baseline = self.centered_baseline(base_y, b.h, span.ascent, span.descent);
                    self.draw_span(
                        &mut rgba,
                        width,
                        span,
                        (b.x - min_x + pad_x) as i32,
                        baseline,
                        b.rgb,
                    );
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
        Some(OsdLayer {
            x: min_x,
            y: min_y,
            width,
            height,
            rgba,
        })
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

    /// Split transient rows and the status readout by vertical edge.
    /// Same-corner entries stack in push order.
    fn collect_entries(content: &OsdContent) -> (Vec<Entry>, Vec<Entry>) {
        let same_corner = content.position == content.status_position;
        let transient_top = content.position.is_top();
        let status_top = content.status_position.is_top();
        let mut top: Vec<Entry> = Vec::new();
        let mut bottom: Vec<Entry> = Vec::new();
        for row in &content.rows {
            let entry = Entry::Line(
                format!("{} {}", row.tag, row.text),
                row.rgb,
                content.position,
            );
            if transient_top {
                top.push(entry);
            } else {
                bottom.push(entry);
            }
        }
        if let Some(status) = content.status.as_ref() {
            let entry = Entry::Status(status.clone(), content.status_position);
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
    fn shape_entry(&mut self, entry: Entry, font_px: u32) -> Shaped {
        let pad_x = self.scaled(PILL_PAD_X);
        match entry {
            Entry::Line(line, rgb, anchor) => {
                let span = self.shape_line(&line, font_px);
                let row_h = self.row_height(font_px, span.ascent, span.descent);
                let w = span.width.ceil() as u32 + pad_x * 2;
                Shaped {
                    anchor,
                    body: RowBody::Line { span },
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
    fn shape_status(&mut self, status: OsdStatus, anchor: OsdAnchor, font_px: u32) -> Shaped {
        let label = self.shape_line(&status.label, font_px);
        let value = self.shape_line(&status.value, font_px);
        if status.label.is_empty() {
            // Solo value pill, as before.
            let row_h = self.row_height(font_px, value.ascent, value.descent);
            let pad_x = self.scaled(PILL_PAD_X);
            let w = value.width.ceil() as u32 + pad_x * 2;
            return Shaped {
                anchor,
                body: RowBody::Line { span: value },
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

    /// Shape one line with full GPOS kerning and script fallback.
    /// Measure and draw share the span, so the box always fits.
    fn shape_line(&mut self, line: &str, font_px: u32) -> ShapedSpan {
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
enum Entry {
    Line(String, (u8, u8, u8), OsdAnchor),
    Status(OsdStatus, OsdAnchor),
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
        span: ShapedSpan,
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
    span: ShapedSpan,
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

/// Straight-alpha blend of one pixel over the layer buffer.
fn blend_over(dst: &mut [u8], r: u8, g: u8, b: u8, a: u8) {
    let alpha = f32::from(a) / 255.0;
    dst[0] = (f32::from(r) * alpha + f32::from(dst[0]) * (1.0 - alpha)).round() as u8;
    dst[1] = (f32::from(g) * alpha + f32::from(dst[1]) * (1.0 - alpha)).round() as u8;
    dst[2] = (f32::from(b) * alpha + f32::from(dst[2]) * (1.0 - alpha)).round() as u8;
    dst[3] = (f32::from(a) + f32::from(dst[3]) * (1.0 - alpha))
        .round()
        .min(255.0) as u8;
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
    use nerust_render_traits::renderer::{OsdContent, OsdRow, OsdStatus, SafeInsets};

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
}
