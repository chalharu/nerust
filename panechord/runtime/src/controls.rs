use std::{collections::BTreeSet, fmt, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextOverflow {
    #[default]
    Clip,
    Wrap,
    EllipsizeMiddle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableColumn {
    pub title: String,
    pub width: Option<f32>,
    pub text: crate::TextStyle,
    pub overflow: TextOverflow,
    pub stretch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub text: String,
    pub foreground: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub key: String,
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Arc<[TableColumn]>,
    pub rows: Arc<[Arc<TableRow>]>,
    pub empty_text: String,
    pub column_gap: f32,
    pub row_gap: f32,
}

impl Table {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.columns.is_empty()
            || self.columns.len() > 64
            || self.rows.len() > 16384
            || self.rows.len() * self.columns.len() > 262144
        {
            return Err("table dimensions exceed limits");
        }
        if [self.column_gap, self.row_gap]
            .into_iter()
            .any(|gap| !gap.is_finite() || !(0.0..=4096.0).contains(&gap))
        {
            return Err("invalid table gap");
        }
        let mut bytes = self.empty_text.len();
        for column in &*self.columns {
            if column
                .width
                .is_some_and(|width| !width.is_finite() || !(1.0..=16384.0).contains(&width))
            {
                return Err("invalid table column width");
            }
            crate::Style {
                text: Some(column.text),
                ..Default::default()
            }
            .validate(crate::Layout::Stack {
                horizontal: false,
                gap: 0.0,
            })?;
            bytes = bytes.saturating_add(column.title.len());
        }
        let mut keys = BTreeSet::new();
        for row in &*self.rows {
            if !crate::valid_item_key(&row.key) || row.key.len() > 256 || !keys.insert(&row.key) {
                return Err("invalid or duplicate table row key");
            }
            if row.cells.len() != self.columns.len() {
                return Err("table row has wrong cell count");
            }
            bytes =
                bytes.saturating_add(row.cells.iter().map(|cell| cell.text.len()).sum::<usize>());
        }
        if bytes > 8 * 1024 * 1024 {
            return Err("table text exceeds limits");
        }
        Ok(())
    }
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn fixture() -> Table {
        Table {
            columns: vec![TableColumn {
                title: "Text".into(),
                width: None,
                text: Default::default(),
                overflow: TextOverflow::Wrap,
                stretch: true,
            }]
            .into(),
            rows: vec![Arc::new(TableRow {
                key: "one".into(),
                cells: vec![TableCell {
                    text: "value".into(),
                    foreground: Some([70, 70, 70]),
                }],
            })]
            .into(),
            ..Default::default()
        }
    }

    #[test]
    fn validates_shape_keys_geometry_and_text_budgets() {
        let table = fixture();
        assert_eq!(table.validate(), Ok(()));
        let mut invalid = table.clone();
        invalid.rows = vec![table.rows[0].clone(), table.rows[0].clone()].into();
        assert_eq!(
            invalid.validate(),
            Err("invalid or duplicate table row key")
        );
        invalid = table.clone();
        invalid.rows = vec![Arc::new(TableRow {
            key: "one".into(),
            cells: vec![],
        })]
        .into();
        assert_eq!(invalid.validate(), Err("table row has wrong cell count"));
        for gap in [f32::NAN, f32::INFINITY, -1.0, 4097.0] {
            invalid = table.clone();
            invalid.column_gap = gap;
            assert_eq!(invalid.validate(), Err("invalid table gap"));
        }
        for width in [f32::NAN, 0.0, 16385.0] {
            invalid = table.clone();
            Arc::make_mut(&mut invalid.columns)[0].width = Some(width);
            assert_eq!(invalid.validate(), Err("invalid table column width"));
        }
        invalid = table.clone();
        invalid.empty_text = "x".repeat(8 * 1024 * 1024 + 1);
        assert_eq!(invalid.validate(), Err("table text exceeds limits"));
        invalid = table.clone();
        invalid.rows = Arc::from([]);
        assert_eq!(invalid.validate(), Ok(()));
        invalid.columns = Arc::from([]);
        assert_eq!(invalid.validate(), Err("table dimensions exceed limits"));
    }
}

impl fmt::Display for Choice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}
pub fn validate_choices(items: &[Choice], selected: &str) -> Result<(), &'static str> {
    if items.len() > 4096 {
        return Err("too many choices");
    }
    let mut ids = BTreeSet::new();
    for item in items {
        if !crate::valid_item_key(&item.id) || !ids.insert(&item.id) {
            return Err("invalid or duplicate choice id");
        }
    }
    if !selected.is_empty() && !items.iter().any(|item| item.id == selected) {
        return Err("unknown selected choice");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}
impl SliderRange {
    pub fn validate(self) -> Result<Self, &'static str> {
        let span = self.max - self.min;
        if !self.min.is_finite()
            || !self.max.is_finite()
            || !self.step.is_finite()
            || self.min.abs() > 1_000_000_000.0
            || self.max.abs() > 1_000_000_000.0
            || span <= 0.0
            || self.step <= 0.0
            || self.step > span
            || !(span / self.step).is_finite()
        {
            return Err("invalid slider range or step");
        }
        Ok(self)
    }
    pub fn normalize(self, value: f64) -> Result<f64, &'static str> {
        self.validate()?;
        if !value.is_finite() || !(self.min..=self.max).contains(&value) {
            return Err("slider value outside range");
        }
        Ok(
            (self.min + ((value - self.min) / self.step).round() * self.step)
                .clamp(self.min, self.max),
        )
    }
}

#[derive(Clone, Debug)]
pub struct ImageFrame {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}
impl PartialEq for ImageFrame {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}
impl ImageFrame {
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Result<Self, &'static str> {
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|size| size.checked_mul(4));
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || bytes.is_none_or(|size| size > 64 * 1024 * 1024 || size != pixels.len())
        {
            return Err("invalid RGBA image dimensions or length");
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &Arc<[u8]> {
        &self.pixels
    }
}
