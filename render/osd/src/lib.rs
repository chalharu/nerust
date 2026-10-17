//! CPU OSD rasterizer: notification rows to a cropped RGBA layer.
//! One implementation shared by every GPU backend; backends only blit.

use std::collections::HashMap;

use ab_glyph::{Font, FontArc, Glyph, GlyphId, PxScale};
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

/// System fonts with lazy fallback. Load once, share everywhere.
pub struct OsdFonts {
    db: fontdb::Database,
    loaded: Vec<FontArc>,
    loaded_ids: Vec<fontdb::ID>,
    face_ids: Vec<fontdb::ID>,
    char_font: HashMap<char, usize>,
}

impl OsdFonts {
    pub fn load() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let face_ids: Vec<fontdb::ID> = db.faces().map(|face| face.id).collect();
        let mut fonts = Self {
            db,
            loaded: Vec::new(),
            loaded_ids: Vec::new(),
            face_ids,
            char_font: HashMap::new(),
        };
        if fonts.load_best_sans().is_none() {
            log::warn!("osd sans query found nothing");
        }
        fonts
    }

    fn load_face(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(position) = self.loaded_ids.iter().position(|face| *face == id) {
            return Some(position);
        }
        if self.loaded.len() >= 8 {
            return None;
        }
        let bytes: Vec<u8> = self.db.with_face_data(id, |data, _index| data.to_vec())?;
        let font = match FontArc::try_from_vec(bytes) {
            Ok(font) => font,
            Err(error) => {
                log::warn!("osd font parse failed: {error}");
                return None;
            }
        };
        self.loaded.push(font);
        self.loaded_ids.push(id);
        Some(self.loaded.len() - 1)
    }

    /// Load the best available sans face. fontdb's SansSerif
    /// defaults to Arial (often absent on Linux), so prefer common
    /// families, then any sans-named face, then the first face.
    fn load_best_sans(&mut self) -> Option<usize> {
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
            if let Some(id) = self.db.query(&query)
                && let Some(index) = self.load_face(id)
            {
                return Some(index);
            }
        }
        let mut names: Vec<String> = self
            .db
            .faces()
            .flat_map(|face| face.families.iter().map(|(family, _)| family.clone()))
            .collect();
        names.sort();
        names.dedup();
        if let Some(name) = names
            .iter()
            .find(|name| name.to_lowercase().contains("sans"))
        {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                ..fontdb::Query::default()
            };
            if let Some(id) = self.db.query(&query)
                && let Some(index) = self.load_face(id)
            {
                return Some(index);
            }
        }
        let first = self.face_ids.first().copied()?;
        self.load_face(first)
    }
    /// Font index holding `c`, loading faces lazily. Primary first,
    /// then CJK-likely families (they sort late in dense font dirs
    /// and would otherwise starve behind the load cap), then rest.
    fn font_for(&mut self, c: char) -> Option<usize> {
        if let Some(&index) = self.char_font.get(&c) {
            return Some(index);
        }
        let order = self.fallback_order();
        for position in order {
            let id = self.face_ids[position];
            let Some(index) = self.load_face(id) else {
                continue;
            };
            if self.loaded[index].glyph_id(c).0 != 0 {
                self.char_font.insert(c, index);
                return Some(index);
            }
        }
        None
    }

    fn fallback_order(&self) -> Vec<usize> {
        fn cjk(name: &str) -> bool {
            let lower = name.to_lowercase();
            lower.contains("cjk")
                || lower.contains("fallback")
                || lower.contains("droid")
                || lower.contains("mincho")
                || lower.contains("gothic")
        }
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for (position, id) in self.face_ids.iter().enumerate() {
            let hit = self
                .db
                .face(*id)
                .is_some_and(|face| face.families.iter().any(|(name, _)| cjk(name)));
            if hit {
                first.push(position);
            } else {
                rest.push(position);
            }
        }
        first.extend(rest);
        first
    }

    pub fn has_fonts(&self) -> bool {
        !self.loaded.is_empty()
    }
}

/// Fixed layout constants shared with the old overlay math.
const MARGIN: u32 = 8;
const ROW_GAP: u32 = 4;
const PILL_PAD_X: u32 = 6;
const PILL_PAD_Y: u32 = 2;
const PILL_RADIUS: u32 = 6;
const PILL_BG: (u8, u8, u8, u8) = (0, 0, 0, 184);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u32,
    glyph: u16,
    px: u32,
    qfrac: u8,
}

struct CachedGlyph {
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
    alpha: Vec<u8>,
}

/// One laid-out glyph: absolute fractional pen plus quantized
/// subpixel shift. Measure and draw share the run, so the box
/// always fits the text.
struct RunGlyph {
    font: usize,
    glyph: u16,
    pen_x: f32,
    qfrac: u8,
}

/// Rasterizer with a glyph cache. Pixels are device pixels.
pub struct OsdRaster {
    fonts: OsdFonts,
    glyphs: HashMap<GlyphKey, CachedGlyph>,
}

impl OsdRaster {
    pub fn new() -> Self {
        Self {
            fonts: OsdFonts::load(),
            glyphs: HashMap::new(),
        }
    }

    pub fn has_fonts(&self) -> bool {
        self.fonts.has_fonts()
    }

    /// Rasterize rows into a cropped layer. `None` when empty/fontless.
    pub fn rasterize(
        &mut self,
        content: &OsdContent,
        surface_w: u32,
        surface_h: u32,
    ) -> Option<OsdLayer> {
        if surface_w == 0 || surface_h == 0 || !self.fonts.has_fonts() {
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
                RowBody::Line { run, ascent } => {
                    let baseline = base_y as f32 + PILL_PAD_Y as f32 + ascent;
                    self.draw_run(
                        &mut rgba,
                        width,
                        run,
                        (b.x - min_x + PILL_PAD_X) as f32,
                        baseline,
                        font_px,
                        b.rgb,
                    );
                }
                RowBody::Status {
                    left,
                    right,
                    ascent,
                } => {
                    let baseline = base_y as f32 + PILL_PAD_Y as f32 + ascent;
                    let base_x = (b.x - min_x + PILL_PAD_X) as f32;
                    self.draw_run(
                        &mut rgba, width, &left.run, base_x, baseline, font_px, b.rgb,
                    );
                    // Right span starts after the left span plus the
                    // shared gap, so no separator is drawn twice.
                    let gap = self.layout_run(" ", font_px).1.max(1.0);
                    let empty = left.run.is_empty() || right.run.is_empty();
                    let right_x = base_x + left.width + if empty { 0.0 } else { gap };
                    self.draw_run(
                        &mut rgba, width, &right.run, right_x, baseline, font_px, b.rgb,
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
        let gap = self.layout_run(" ", font_px).1.max(1.0);
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
                        let (run, width, ascent, descent) = raster.layout_run(&line, font_px);
                        let row_h = raster.row_height(font_px, ascent, descent);
                        let w = width.ceil() as u32 + PILL_PAD_X * 2;
                        let body = RowBody::Line { run, ascent };
                        (anchor, body, rgb, w, row_h)
                    }
                    Entry::Status(status, anchor) => {
                        let (label_run, label_w, ascent_l, _) =
                            raster.layout_run(&status.label, font_px);
                        let (value_run, value_w, ascent_v, descent_v) =
                            raster.layout_run(&status.value, font_px);
                        let ascent = ascent_l.max(ascent_v);
                        let row_h = raster.row_height(font_px, ascent, descent_v);
                        // Value always on the outside (margin side) so the
                        // digits never shift: left anchors read
                        // "value label", right anchors "label value".
                        let outer_left = matches!(
                            anchor,
                            NotificationPosition::TopLeft | NotificationPosition::BottomLeft
                        );
                        let (left, right) = if outer_left {
                            (
                                StatusSpan {
                                    run: value_run,
                                    width: value_w,
                                },
                                StatusSpan {
                                    run: label_run,
                                    width: label_w,
                                },
                            )
                        } else {
                            (
                                StatusSpan {
                                    run: label_run,
                                    width: label_w,
                                },
                                StatusSpan {
                                    run: value_run,
                                    width: value_w,
                                },
                            )
                        };
                        let middle = if status.label.is_empty() || status.value.is_empty() {
                            0.0
                        } else {
                            gap
                        };
                        let w = (left.width + middle + right.width).ceil() as u32 + PILL_PAD_X * 2;
                        let body = RowBody::Status {
                            left,
                            right,
                            ascent,
                        };
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

    /// Lay out one line into absolute-pen glyphs with kerning.
    /// Empty outlines (spaces) still advance the pen; the pen stays
    /// fractional and each glyph carries its own quantized subpixel
    /// shift, so spacing never jitters.
    fn layout_run(&mut self, line: &str, font_px: u32) -> (Vec<RunGlyph>, f32, f32, f32) {
        let mut run = Vec::new();
        let mut pen = 0f32;
        let mut ascent = 0f32;
        let mut descent = 0f32;
        let mut prev: Option<(usize, u16)> = None;
        for c in line.chars() {
            let Some(index) = self.fonts.font_for(c) else {
                prev = None;
                continue;
            };
            let font = &self.fonts.loaded[index];
            let units = font.units_per_em().unwrap_or(2048.0);
            let factor = font_px as f32 / units;
            let id = font.glyph_id(c);
            if let Some((pi, pid)) = prev
                && pi == index
            {
                pen += font.kern_unscaled(GlyphId(pid), id) * factor;
            }
            let frac = pen - pen.floor();
            let qfrac = ((frac * 4.0).round() as i32).clamp(0, 4) as u8;
            run.push(RunGlyph {
                font: index,
                glyph: id.0,
                pen_x: pen,
                qfrac,
            });
            pen += font.h_advance_unscaled(id) * factor;
            ascent = ascent.max(font.ascent_unscaled() * factor);
            descent = descent.min(font.descent_unscaled() * factor);
            prev = Some((index, id.0));
        }
        (run, pen, ascent, descent)
    }

    fn row_height(&self, font_px: u32, ascent: f32, descent: f32) -> u32 {
        let content_h = (ascent - descent).ceil().max(1.0) as u32;
        (font_px * 16 / 10 + PILL_PAD_Y * 2).max(content_h + PILL_PAD_Y * 2)
    }

    /// Blit one laid-out run at an integer base position.
    #[allow(clippy::too_many_arguments)]
    fn draw_run(
        &mut self,
        rgba: &mut [u8],
        stride_w: u32,
        run: &[RunGlyph],
        x_base: f32,
        baseline: f32,
        font_px: u32,
        rgb: (u8, u8, u8),
    ) {
        let scale = PxScale::from(font_px as f32);
        for g in run {
            let key = GlyphKey {
                font: g.font as u32,
                glyph: g.glyph,
                px: font_px,
                qfrac: g.qfrac,
            };
            let cached = if let Some(cached) = self.glyphs.get(&key) {
                cached
            } else {
                let font = &self.fonts.loaded[g.font];
                let id = GlyphId(g.glyph);
                let outlined = font.outline_glyph(Glyph {
                    id,
                    scale,
                    position: ab_glyph::Point {
                        x: f32::from(g.qfrac) / 4.0,
                        y: 0.0,
                    },
                });
                let Some(outlined) = outlined else {
                    // No outline (e.g. space): advance already counted
                    // in the run, so record an empty slot and move on.
                    self.glyphs.insert(
                        key,
                        CachedGlyph {
                            width: 0,
                            height: 0,
                            dx: 0.0,
                            dy: 0.0,
                            alpha: Vec::new(),
                        },
                    );
                    continue;
                };
                let bounds = outlined.px_bounds();
                let w = bounds.width() as u32;
                let h = bounds.height() as u32;
                let mut alpha = vec![0u8; w as usize * h as usize];
                outlined.draw(|x, y, coverage| {
                    alpha[y as usize * w as usize + x as usize] = (coverage * 255.0).round() as u8;
                });
                self.glyphs.insert(
                    key,
                    CachedGlyph {
                        width: w,
                        height: h,
                        dx: bounds.min.x,
                        dy: bounds.min.y,
                        alpha,
                    },
                );
                &self.glyphs[&key]
            };
            blit_glyph(
                rgba,
                stride_w,
                &cached.alpha,
                cached.width,
                cached.height,
                (x_base + g.pen_x.floor() + cached.dx).round() as i32,
                (baseline + cached.dy).round() as i32,
                rgb,
            );
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
    Line {
        run: Vec<RunGlyph>,
        ascent: f32,
    },
    Status {
        left: StatusSpan,
        right: StatusSpan,
        ascent: f32,
    },
}

struct StatusSpan {
    run: Vec<RunGlyph>,
    width: f32,
}

fn is_top(position: NotificationPosition) -> bool {
    matches!(
        position,
        NotificationPosition::TopLeft | NotificationPosition::TopRight
    )
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

/// Blit one cached glyph bitmap at integer layer coordinates.
#[allow(clippy::too_many_arguments)]
fn blit_glyph(
    rgba: &mut [u8],
    stride_w: u32,
    alpha: &[u8],
    glyph_w: u32,
    glyph_h: u32,
    x: i32,
    y: i32,
    rgb: (u8, u8, u8),
) {
    if glyph_w == 0 || glyph_h == 0 {
        return;
    }
    for row in 0..glyph_h {
        for col in 0..glyph_w {
            let coverage = alpha[row as usize * glyph_w as usize + col as usize];
            if coverage == 0 {
                continue;
            }
            let (dx, dy) = (x + col as i32, y + row as i32);
            if dx < 0 || dy < 0 {
                continue;
            }
            let offset = (dy as u32 * stride_w + dx as u32) as usize * 4;
            if let Some(pixel) = rgba.get_mut(offset..offset + 4) {
                blend_over(pixel, rgb.0, rgb.1, rgb.2, coverage);
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
        let (_, plain_w, _, _) = raster.layout_run("ab", 16);
        let (_, spaced_w, _, _) = raster.layout_run("a b", 16);
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
        let gap = raster.layout_run(" ", 16).1;
        let label_w = raster.layout_run("Paused", 16).1;
        // Left anchor: value first, so the digit block starts at the pad.
        let running = raster
            .rasterize(&content("", "60.1 fps", left), 960, 720)
            .expect("status layer");
        let value_w = raster.layout_run("60.1 fps", 16).1;
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
