//! CPU OSD rasterizer: notification rows to a cropped RGBA layer.
//! One implementation shared by every GPU backend; backends only blit.

use std::collections::HashMap;

use ab_glyph::{Font, FontArc, Glyph, PxScale};
use nerust_render_traits::renderer::OsdLayer;
use nerust_settings_core::model::local::NotificationPosition;

/// One pill row: tag plus body in a single color.
#[derive(Debug, Clone)]
pub struct OsdRow {
    pub tag: String,
    pub text: String,
    pub rgb: (u8, u8, u8),
}

/// Raster input: transient rows plus optional status, with corners.
#[derive(Debug, Clone, Default)]
pub struct OsdContent {
    pub rows: Vec<OsdRow>,
    pub status: Option<OsdRow>,
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
    /// Font index holding `c`, loading faces lazily. Primary first.
    fn font_for(&mut self, c: char) -> Option<usize> {
        if let Some(&index) = self.char_font.get(&c) {
            return Some(index);
        }
        for position in 0..self.face_ids.len() {
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
}

struct CachedGlyph {
    width: u32,
    height: u32,
    dx: i32,
    dy: i32,
    advance: f32,
    alpha: Vec<u8>,
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
            self.draw_text(
                &mut rgba,
                width,
                &b.line,
                b.x - min_x + PILL_PAD_X,
                b.y - min_y,
                font_px,
                b.rgb,
            );
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
        // Each line carries its own anchor corner for split placement.
        let mut top: Vec<(String, (u8, u8, u8), NotificationPosition)> = Vec::new();
        let mut bottom: Vec<(String, (u8, u8, u8), NotificationPosition)> = Vec::new();
        for row in &content.rows {
            let entry = (
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
            let entry = (status.text.clone(), status.rgb, content.status_position);
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
        let mut boxes = Vec::new();
        let mut place = |list: Vec<(String, (u8, u8, u8), NotificationPosition)>,
                         from_top: bool,
                         raster: &mut Self| {
            let mut cursor = if from_top {
                MARGIN
            } else {
                surface_h.saturating_sub(MARGIN)
            };
            let mut ordered = list;
            if !from_top {
                ordered.reverse();
            }
            for (line, rgb, anchor) in ordered {
                let (text_w, row_h) = raster.measure(&line, font_px);
                let w = text_w + PILL_PAD_X * 2;
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
                    line,
                    rgb,
                });
            }
        };
        place(top, true, self);
        place(bottom, false, self);
        Some(boxes)
    }

    fn measure(&mut self, line: &str, font_px: u32) -> (u32, u32) {
        let mut width = 0f32;
        let mut ascent = 0f32;
        let mut descent = 0f32;
        for c in line.chars() {
            let Some(index) = self.fonts.font_for(c) else {
                continue;
            };
            let font = &self.fonts.loaded[index];
            let units = font.units_per_em().unwrap_or(2048.0);
            let factor = font_px as f32 / units;
            width += font.h_advance_unscaled(font.glyph_id(c)) * factor;
            ascent = ascent.max(font.ascent_unscaled() * factor);
            descent = descent.min(font.descent_unscaled() * factor);
        }
        let content_h = (ascent - descent).ceil().max(1.0) as u32;
        let row_h = (font_px * 16 / 10 + PILL_PAD_Y * 2).max(content_h + PILL_PAD_Y * 2);
        (width.ceil().max(1.0) as u32, row_h)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        rgba: &mut [u8],
        stride_w: u32,
        line: &str,
        x0: u32,
        y0: u32,
        font_px: u32,
        rgb: (u8, u8, u8),
    ) {
        let scale = PxScale::from(font_px as f32);
        let Some(primary) = self.fonts.loaded.first() else {
            return;
        };
        let units = primary.units_per_em().unwrap_or(2048.0);
        let ascent = primary.ascent_unscaled() * font_px as f32 / units;
        let baseline = y0 as f32 + PILL_PAD_Y as f32 + ascent;
        let mut pen = x0 as f32;
        for c in line.chars() {
            let Some(index) = self.fonts.font_for(c) else {
                continue;
            };
            let key = GlyphKey {
                font: index as u32,
                glyph: self.fonts.loaded[index].glyph_id(c).0,
                px: font_px,
            };
            if !self.glyphs.contains_key(&key) {
                let font = &self.fonts.loaded[index];
                let id = font.glyph_id(c);
                let glyph = Glyph {
                    id,
                    scale,
                    position: ab_glyph::Point { x: 0.0, y: 0.0 },
                };
                let Some(outlined) = font.outline_glyph(glyph) else {
                    continue;
                };
                let bounds = outlined.px_bounds();
                let w = bounds.width() as u32;
                let h = bounds.height() as u32;
                let mut alpha = vec![0u8; w as usize * h as usize];
                outlined.draw(|x, y, coverage| {
                    alpha[y as usize * w as usize + x as usize] = (coverage * 255.0).round() as u8;
                });
                let units = font.units_per_em().unwrap_or(2048.0);
                let advance = font.h_advance_unscaled(id) * font_px as f32 / units;
                self.glyphs.insert(
                    key,
                    CachedGlyph {
                        width: w,
                        height: h,
                        dx: bounds.min.x.round() as i32,
                        dy: bounds.min.y.round() as i32,
                        advance,
                        alpha,
                    },
                );
            }
            let g = &self.glyphs[&key];
            blit_glyph(
                rgba,
                stride_w,
                &g.alpha,
                g.width,
                g.height,
                (pen + g.dx as f32).round() as i32,
                (baseline + g.dy as f32).round() as i32,
                rgb,
            );
            pen += g.advance;
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
    line: String,
    rgb: (u8, u8, u8),
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
        let (_, row_h) = raster.measure("[INFO] Paused", 16);
        assert_eq!(row_h, 16 * 16 / 10 + 4);
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
