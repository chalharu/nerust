//! CPU OSD rasterizer: notification rows to a cropped RGBA layer.
//! One implementation shared by every GPU backend; backends only blit.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use cosmic_text::{CacheKey, SwashContent};
use nerust_render_traits::renderer::OsdLayer;
use nerust_settings_core::model::local::NotificationPosition;

/// One pill row: tag plus body in a single color.
#[derive(Debug, Clone)]
pub struct OsdRow {
    pub tag: String,
    pub text: String,
    pub rgb: (u8, u8, u8),
}

/// Status readout: label on the left, value right-aligned in a
/// fixed-width pill so digits never shift.
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
    pub position: NotificationPosition,
    pub status_position: NotificationPosition,
}

/// System font preference. cosmic-text shapes and rasterizes;
/// this only picks the sans family (Arial is often absent on
/// Linux, so prefer common families, then any sans-named face).
pub struct OsdFonts {
    sans_name: String,
    has_fonts: bool,
}

impl OsdFonts {
    pub fn load() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let has_fonts = !db.is_empty();
        let sans_name = Self::best_sans_name(&db).unwrap_or_default();
        if sans_name.is_empty() {
            log::warn!("osd sans query found nothing");
        }
        Self {
            sans_name,
            has_fonts,
        }
    }

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

    pub fn has_fonts(&self) -> bool {
        self.has_fonts
    }
}

/// Fixed layout constants shared with the old overlay math.
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
}

impl OsdRaster {
    pub fn new() -> Self {
        let fonts = OsdFonts::load();
        let font_system = FontSystem::new();
        let has_fonts = fonts.has_fonts();
        Self {
            font_system,
            swash: SwashCache::new(),
            sans_name: fonts.sans_name,
            has_fonts,
        }
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
        let font_px = (f64::from(surface_h) * 0.022).ceil().max(6.0) as u32;
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
        for b in &boxes {
            draw_pill(&mut rgba, width, b.x - min_x, b.y - min_y, b.w, b.h);
            let base_y = b.y - min_y;
            match &b.body {
                RowBody::Line { span } => {
                    let baseline = centered_baseline(base_y, b.h, span.ascent, span.descent);
                    self.draw_span(
                        &mut rgba,
                        width,
                        span,
                        (b.x - min_x + PILL_PAD_X) as i32,
                        baseline,
                        b.rgb,
                    );
                }
                RowBody::Status { left, right } => {
                    let ascent = left.span.ascent.max(right.span.ascent);
                    let descent = left.span.descent.min(right.span.descent);
                    let baseline = centered_baseline(base_y, b.h, ascent, descent);
                    let base_x = (b.x - min_x + PILL_PAD_X) as i32;
                    self.draw_span(&mut rgba, width, &left.span, base_x, baseline, b.rgb);
                    // Right span starts after the left span plus the
                    // shared gap, so no separator is drawn twice.
                    let gap = self.shape_line(" ", font_px).width.max(1.0);
                    let empty = left.span.glyphs.is_empty() || right.span.glyphs.is_empty();
                    let right_x = base_x
                        + left.span.width.ceil() as i32
                        + if empty { 0 } else { gap.ceil() as i32 };
                    self.draw_span(&mut rgba, width, &right.span, right_x, baseline, b.rgb);
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
        let same_corner = content.position == content.status_position;
        let transient_top = is_top(content.position);
        let status_top = is_top(content.status_position);
        // Each entry carries its own anchor corner for split placement.
        enum Entry {
            Line(String, (u8, u8, u8), NotificationPosition),
            Status(OsdStatus, NotificationPosition),
        }
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
        if top.is_empty() && bottom.is_empty() {
            return None;
        }
        // Space advance doubles as the label/value separator.
        let gap = self.shape_line(" ", font_px).width.max(1.0);
        let mut boxes = Vec::new();
        let mut place = |list: Vec<Entry>, from_top: bool, raster: &mut Self| {
            let mut cursor = if from_top {
                MARGIN
            } else {
                surface_h.saturating_sub(MARGIN)
            };
            let mut ordered = list;
            if !from_top {
                ordered.reverse();
            }
            for entry in ordered {
                let (anchor, body, rgb, w, row_h) = match entry {
                    Entry::Line(line, rgb, anchor) => {
                        let span = raster.shape_line(&line, font_px);
                        let row_h = raster.row_height(font_px, span.ascent, span.descent);
                        let w = span.width.ceil() as u32 + PILL_PAD_X * 2;
                        let body = RowBody::Line { span };
                        (anchor, body, rgb, w, row_h)
                    }
                    Entry::Status(status, anchor) => {
                        let label = raster.shape_line(&status.label, font_px);
                        let value = raster.shape_line(&status.value, font_px);
                        let ascent = label.ascent.max(value.ascent);
                        let row_h = raster.row_height(font_px, ascent, value.descent);
                        // Value always on the outside (margin side) so the
                        // digits never shift: left anchors read
                        // "value label", right anchors "label value".
                        let outer_left = matches!(
                            anchor,
                            NotificationPosition::TopLeft | NotificationPosition::BottomLeft
                        );
                        let (left, right) = if outer_left {
                            (StatusSpan { span: value }, StatusSpan { span: label })
                        } else {
                            (StatusSpan { span: label }, StatusSpan { span: value })
                        };
                        let middle = if status.label.is_empty() || status.value.is_empty() {
                            0.0
                        } else {
                            gap
                        };
                        let w = (left.span.width + middle + right.span.width).ceil() as u32
                            + PILL_PAD_X * 2;
                        let body = RowBody::Status { left, right };
                        (anchor, body, status.rgb, w, row_h)
                    }
                };
                let x = match anchor {
                    NotificationPosition::TopLeft | NotificationPosition::BottomLeft => MARGIN,
                    _ => surface_w.saturating_sub(w + MARGIN),
                };
                let y = if from_top {
                    let y = cursor;
                    cursor += row_h + ROW_GAP;
                    y
                } else {
                    cursor = cursor.saturating_sub(row_h);
                    let y = cursor;
                    cursor = cursor.saturating_sub(ROW_GAP);
                    y
                };
                boxes.push(PlacedRow {
                    x,
                    y,
                    w,
                    h: row_h,
                    rgb,
                    body,
                });
            }
        };
        place(top, true, self);
        place(bottom, false, self);
        Some(boxes)
    }

    /// Shape one line with full GPOS kerning and script fallback.
    /// Measure and draw share the span, so the box always fits.
    fn shape_line(&mut self, line: &str, font_px: u32) -> ShapedSpan {
        let size = font_px as f32;
        let metrics = Metrics::new(size, size);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, Some(8192.0), None);
        let family = if self.sans_name.is_empty() {
            Family::SansSerif
        } else {
            Family::Name(&self.sans_name)
        };
        buffer.set_text(
            &mut self.font_system,
            line,
            &Attrs::new().family(family),
            Shaping::Advanced,
            None,
        );
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
        let content_h = (ascent - descent).ceil().max(1.0) as u32;
        (font_px * 16 / 10 + PILL_PAD_Y * 2).max(content_h + PILL_PAD_Y * 2)
    }

    /// Blit one shaped span at an integer base position.
    /// Glyph images come from the shared swash cache, keyed by
    /// shaping output, so hinting stays consistent everywhere.
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

enum RowBody {
    Line { span: ShapedSpan },
    Status { left: StatusSpan, right: StatusSpan },
}

struct StatusSpan {
    span: ShapedSpan,
}

fn is_top(position: NotificationPosition) -> bool {
    matches!(
        position,
        NotificationPosition::TopLeft | NotificationPosition::TopRight
    )
}

/// Baseline that centers the content block in the pill. The pill is
/// taller than the text, so top-anchoring leaves the bottom empty.
fn centered_baseline(base_y: u32, row_h: u32, ascent: f32, descent: f32) -> i32 {
    let content_h = ascent - descent;
    let extra = row_h as f32 - (content_h + PILL_PAD_Y as f32 * 2.0);
    base_y as i32 + PILL_PAD_Y as i32 + (extra / 2.0).round() as i32 + ascent as i32
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
fn draw_pill(rgba: &mut [u8], stride_w: u32, x: u32, y: u32, w: u32, h: u32) {
    let radius = PILL_RADIUS.min(w / 2).min(h / 2) as f32;
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

    fn content() -> OsdContent {
        OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]".to_string(),
                text: "Paused".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: NotificationPosition::BottomLeft,
            status_position: NotificationPosition::TopLeft,
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
    fn row_height_follows_the_shared_formula() {
        let mut raster = OsdRaster::new();
        if !raster.has_fonts() {
            eprintln!("SKIP: no system fonts");
            return;
        }
        // 720p: font 16px, row 16*16/10+4 = 29.
        let content = OsdContent {
            rows: vec![OsdRow {
                tag: "[INFO]".to_string(),
                text: "Paused".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: NotificationPosition::BottomLeft,
            status_position: NotificationPosition::TopLeft,
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
        top_right.position = NotificationPosition::TopRight;
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
        // Full GPOS shaping must tighten kerned pairs versus the
        // sum of isolated advances. Skip when the system font
        // lacks this pair instead of asserting font specifics.
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
        let content = super::OsdContent {
            rows: vec![super::OsdRow {
                tag: "[INFO]".to_string(),
                text: "一時停止しました".to_string(),
                rgb: (255, 255, 255),
            }],
            status: None,
            position: super::NotificationPosition::BottomLeft,
            status_position: super::NotificationPosition::TopLeft,
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
        let body = super::OsdContent {
            rows: vec![super::OsdRow {
                tag: String::new(),
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
        let content = |label: &str, value: &str, anchor: NotificationPosition| OsdContent {
            rows: Vec::new(),
            status: Some(super::OsdStatus {
                label: label.to_string(),
                value: value.to_string(),
                rgb: (200, 200, 200),
            }),
            position: super::NotificationPosition::BottomLeft,
            status_position: anchor,
        };
        let left = super::NotificationPosition::BottomLeft;
        let right = super::NotificationPosition::BottomRight;
        let gap = raster.shape_line(" ", 16).width;
        let label_w = raster.shape_line("Paused", 16).width;
        // Left anchor: value first, so the digit block starts at the pad.
        let running = raster
            .rasterize(&content("", "60.1 fps", left), 960, 720)
            .expect("status layer");
        let value_w = raster.shape_line("60.1 fps", 16).width;
        assert_eq!(running.width, value_w.ceil() as u32 + super::PILL_PAD_X * 2);
        // Paused adds label plus one gap, symmetric on both anchors.
        let paused_left = raster
            .rasterize(&content("Paused", "60.1 fps", left), 960, 720)
            .expect("status layer");
        let paused_right = raster
            .rasterize(&content("Paused", "60.1 fps", right), 960, 720)
            .expect("status layer");
        let expect = (value_w + gap + label_w).ceil() as u32 + super::PILL_PAD_X * 2;
        assert_eq!(paused_left.width, expect);
        assert_eq!(paused_right.width, expect);
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
