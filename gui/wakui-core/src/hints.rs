//! Framework layout hints. All fields are `Option`; `None` means
//! native default / automatic measurement. View definitions never set
//! absolute pixels here; measure/arrange stays on the native side.
//!
//! `Rect` is a framework primitive for the future `AbsolutePolicy`.
//! Toolkit types are never named here.

/// Physical-edge-agnostic margin/padding hint.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Thickness {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// MAUI `LayoutOptions` analog: placement request, never a pixel value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutAlign {
    Start,
    Center,
    End,
    Fill,
}

/// Framework primitive rectangle. Only `AbsolutePolicy` reads it;
/// ordinary policies ignore it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Optional placement requests carried by metadata (`ControlMetadata.hints`).
/// `None` means native default / automatic measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LayoutHints {
    pub margin: Option<Thickness>,
    /// `None` means auto.
    pub width_request: Option<f64>,
    /// `None` means auto.
    pub height_request: Option<f64>,
    pub h_align: Option<LayoutAlign>,
    pub v_align: Option<LayoutAlign>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_none() {
        let hints = LayoutHints::default();
        assert_eq!(
            hints,
            LayoutHints {
                margin: None,
                width_request: None,
                height_request: None,
                h_align: None,
                v_align: None,
            }
        );
    }
}
