//! Extensible leaf controls (trait objects).
//!
//! `Control` stores a boxed [`ControlLeaf`]; base leaves and the generic
//! [`ListLeaf`] are framework-provided, domain leaves are implemented
//! outside `wakui`. Flavors render through [`ControlVisitor`] plus `as_any`
//! downcasting and never match on view-definition types.
//!
//! Dependency direction: leaves and containers live in `wakui` and know
//! only framework types. View definitions implement leaves outside
//! `wakui`; flavors render through `ControlVisitor` plus `as_any`
//! downcasting. `debug_name` is diagnostics only, never a dispatch key.

use std::any::Any;

use crate::control::{Control, ControlId, ControlInput, ControlVisitor};
use crate::hints::LayoutHints;

/// Render core. Flavors depend only on this; cloning and diffing are
/// separate traits so render-only clients are not forced to implement them.
pub trait Leaf: std::fmt::Debug + Send + 'static {
    /// Expose nested controls (only container-like leaves override this).
    fn walk(&self, ctx: &LeafCtx, visitor: &mut dyn ControlVisitor) {
        let _ = (ctx, visitor);
    }
    /// Structural input validation. Enabled/visible gating stays in
    /// `Control::accepts`; domain validity stays view-side.
    fn accepts(&self, _input: &ControlInput) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any;
    /// Diagnostics only. Dispatch via `as_any` + `TypeId`, never this string.
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// What `Control` stores: cloneable and diffable leaves. Only `clone_box`
/// is mandatory; `equals`/`updates` degrade to safe defaults so a missing
/// override stays correct, only slower (full refresh).
pub trait ControlLeaf: Leaf {
    fn clone_box(&self) -> Box<dyn ControlLeaf>;
    fn equals(&self, other: &dyn Leaf) -> bool {
        let _ = other;
        false
    }
    fn updates(&self, old: Option<&dyn Leaf>) -> Vec<LeafUpdate> {
        let _ = old;
        vec![LeafUpdate::FullRefresh]
    }
}

/// Framework primitive context for `walk`: id plus hints only.
/// No domain or native types travel here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafCtx {
    pub id: ControlId,
    pub hints: LayoutHints,
}

/// Framework-primitive leaf diff. Any value change collapses into
/// `ValueChanged`; flavors re-read the new leaf. No leaf payload,
/// native type, or domain type is named here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafUpdate {
    ValueChanged,
    FullRefresh,
}

/// Generic column definition for `ListLeaf`. Headers arrive
/// pre-localized; widths are optional hints (`None` = auto).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub id: String,
    pub header: String,
    pub width: Option<f64>,
}

/// One generic row. `depth == 0` is a flat list, `depth > 0` indents as a
/// tree; `expanded == None` marks a leaf row. Cells reuse existing leaves
/// so no per-domain enum grows.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    /// Stable framework id minted from the view-side key.
    pub id: ControlId,
    pub depth: u16,
    pub expanded: Option<bool>,
    pub cells: Vec<Control>,
}

/// Framework-provided generic list. Covers list/table/tree views with one
/// type; domain-specific row kinds are added as external leaves instead.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListLeaf {
    pub columns: Vec<ColumnDef>,
    pub rows: Vec<ItemRow>,
}

impl Leaf for ListLeaf {
    fn walk(&self, ctx: &LeafCtx, visitor: &mut dyn ControlVisitor) {
        // Degraded fallback until flavors gain virtualized item rendering:
        // expose cells as flat controls, preserving order and visibility.
        // Placement may degrade; content and order must not.
        let _ = ctx;
        for row in &self.rows {
            for cell in &row.cells {
                visitor.control(cell);
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn debug_name(&self) -> &'static str {
        "ListLeaf"
    }
}

impl ControlLeaf for ListLeaf {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlDisplay, ControlMetadata};

    fn cell(label: &str) -> Control {
        Control {
            id: ControlId::fresh(),
            label: label.into(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(NoteLeaf { text: label.into() }),
        }
    }

    fn ctx() -> LeafCtx {
        LeafCtx {
            id: ControlId::fresh(),
            hints: LayoutHints::default(),
        }
    }

    #[test]
    fn list_leaf_is_object_safe_and_storable() {
        let leaf = ListLeaf {
            columns: vec![ColumnDef {
                id: "c".into(),
                header: "H".into(),
                width: None,
            }],
            rows: vec![ItemRow {
                id: ControlId::fresh(),
                depth: 0,
                expanded: None,
                cells: vec![cell("a")],
            }],
        };
        let boxed: Box<dyn ControlLeaf> = Box::new(leaf);
        assert_eq!(boxed.debug_name(), "ListLeaf");
        let cloned = boxed.clone_box();
        assert!(boxed.as_any().downcast_ref::<ListLeaf>().is_some());
        assert!(cloned.as_any().downcast_ref::<ListLeaf>().is_some());
    }

    #[test]
    fn equals_is_false_across_types_and_true_for_same_value() {
        struct Other;
        impl std::fmt::Debug for Other {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Other")
            }
        }
        impl Leaf for Other {
            fn walk(&self, _ctx: &LeafCtx, _visitor: &mut dyn ControlVisitor) {}
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
        impl ControlLeaf for Other {
            fn clone_box(&self) -> Box<dyn ControlLeaf> {
                Box::new(Other)
            }
        }
        let leaf = ListLeaf::default();
        let other = Other;
        assert!(!ControlLeaf::equals(&leaf, &other as &dyn Leaf));
        assert!(ControlLeaf::equals(&leaf, &leaf as &dyn Leaf));
    }

    #[test]
    fn walk_degrades_to_flat_cells_without_reordering() {
        struct Recorder(Vec<String>);
        impl ControlVisitor for Recorder {
            fn control(&mut self, control: &Control) {
                self.0.push(control.label.clone());
            }
        }
        let leaf = ListLeaf {
            columns: Vec::new(),
            rows: vec![
                ItemRow {
                    id: ControlId::fresh(),
                    depth: 1,
                    expanded: Some(true),
                    cells: vec![cell("first"), cell("second")],
                },
                ItemRow {
                    id: ControlId::fresh(),
                    depth: 0,
                    expanded: None,
                    cells: vec![cell("third")],
                },
            ],
        };
        let mut recorder = Recorder(Vec::new());
        leaf.walk(&ctx(), &mut recorder);
        assert_eq!(recorder.0, vec!["first", "second", "third"]);
    }

    #[test]
    fn updates_need_no_old_for_full_refresh_and_empty_when_same() {
        let leaf = ListLeaf::default();
        assert_eq!(
            ControlLeaf::updates(&leaf, None),
            vec![LeafUpdate::FullRefresh]
        );
        assert!(ControlLeaf::updates(&leaf, Some(&leaf as &dyn Leaf)).is_empty());
        let mut changed = ListLeaf::default();
        changed.rows.push(ItemRow {
            id: ControlId::fresh(),
            depth: 0,
            expanded: None,
            cells: vec![cell("x")],
        });
        assert_eq!(
            ControlLeaf::updates(&changed, Some(&leaf as &dyn Leaf)),
            vec![LeafUpdate::ValueChanged]
        );
    }
}

// --- Framework-provided base leaves (the closed base set) ---
//
// Domain-specific leaves are added outside `wakui` as further `impl Leaf`s.
// Each base leaf owns its data, validates structurally via `accepts`, and
// diffs via field comparison.

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
