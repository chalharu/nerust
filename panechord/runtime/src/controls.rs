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
