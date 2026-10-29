//! Framework-provided base leaves (the closed base set).
//!
// Domain-specific leaves are added outside `wakui` as further `impl Leaf`s.
// Each base leaf owns its data, validates structurally via `accepts`, and
// diffs via field comparison.

use std::any::Any;

use crate::scene::control::ControlInput;

use super::core::{ControlLeaf, Leaf, LeafUpdate};

/// On/off switch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToggleLeaf {
    pub value: bool,
}

/// Single-choice picker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChoiceLeaf {
    pub choices: Vec<String>,
    pub selected: Option<usize>,
}

/// Bounded numeric input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLeaf {
    pub value: u32,
    pub min: u32,
    pub max: u32,
    pub step: u32,
    pub unit: String,
    pub formatted: String,
}

/// Free text input. `browse_label.is_some()` means Activate opens a picker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextLeaf {
    pub value: String,
    pub placeholder: String,
    pub browse_label: Option<String>,
}

/// Key-capture binding row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingLeaf {
    pub text: String,
    pub capturing: bool,
    pub change_label: String,
    pub clear_label: String,
}

/// Push button.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActionLeaf;

/// Read-only label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteLeaf {
    pub text: String,
}

/// Generic raster image (RGBA8, row-major). Flavors render verbatim and
/// report pointer positions as `ControlInput::Hover`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageLeaf {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

macro_rules! base_leaf {
    ($t:ty) => {
        impl ControlLeaf for $t {
            fn clone_box(&self) -> Box<dyn ControlLeaf> {
                Box::new(self.clone())
            }
            fn equals(&self, other: &dyn Leaf) -> bool {
                other.as_any().downcast_ref::<Self>() == Some(self)
            }
            fn updates(&self, old: Option<&dyn Leaf>) -> Vec<LeafUpdate> {
                match old.and_then(|o| o.as_any().downcast_ref::<Self>()) {
                    None => vec![LeafUpdate::FullRefresh],
                    Some(prev) if prev != self => vec![LeafUpdate::ValueChanged],
                    Some(_) => Vec::new(),
                }
            }
        }
    };
}

base_leaf!(ToggleLeaf);
base_leaf!(ChoiceLeaf);
base_leaf!(NumberLeaf);
base_leaf!(TextLeaf);
base_leaf!(BindingLeaf);
base_leaf!(ActionLeaf);
base_leaf!(NoteLeaf);
impl Leaf for NoteLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
base_leaf!(ImageLeaf);

impl Leaf for ToggleLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        matches!(input, ControlInput::Bool(_))
    }
}

impl Leaf for ChoiceLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        match input {
            ControlInput::Select(index) => *index < self.choices.len(),
            _ => false,
        }
    }
}

impl Leaf for NumberLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        match input {
            ControlInput::Number(n) => (self.min..=self.max).contains(n),
            _ => false,
        }
    }
}

impl Leaf for TextLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        match input {
            ControlInput::Text(_) => true,
            ControlInput::Activate => self.browse_label.is_some(),
            _ => false,
        }
    }
}

impl Leaf for BindingLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        match input {
            ControlInput::Activate | ControlInput::CaptureClear => true,
            ControlInput::CaptureKey(_) => self.capturing,
            _ => false,
        }
    }
}

impl Leaf for ActionLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        matches!(input, ControlInput::Activate)
    }
}

impl Leaf for ImageLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn accepts(&self, input: &ControlInput) -> bool {
        matches!(input, ControlInput::Hover { .. })
    }
}
