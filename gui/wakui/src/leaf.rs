//! Extensible leaf controls (trait objects).
//!
//! Phase 1 skeleton: introduces the traits and the framework-provided
//! `ListLeaf` without touching the existing `ControlKind` enum.
//! Existing screens keep building `ControlKind`; migration to
//! `impl Leaf` happens in Phase 2.
//!
//! Dependency direction: leaves and containers live in `wakui` and know
//! only framework types. View definitions implement leaves outside
//! `wakui`; flavors render through `ControlVisitor` plus `as_any`
//! downcasting. `debug_name` is diagnostics only, never a dispatch key.

use std::any::Any;

use crate::control::{Control, ControlId, ControlVisitor};
use crate::hints::LayoutHints;

/// Render core. Flavors depend only on this; cloning and diffing are
/// separate traits so render-only clients are not forced to implement them.
pub trait Leaf: std::fmt::Debug + Send + 'static {
    fn walk(&self, ctx: &LeafCtx, visitor: &mut dyn ControlVisitor);
    fn as_any(&self) -> &dyn Any;
    /// Diagnostics only. Dispatch via `as_any` + `TypeId`, never this string.
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Required to store a leaf in the owned snapshot (`Control`).
pub trait LeafClone: Leaf {
    fn clone_box(&self) -> Box<dyn LeafClone>;
}

/// Required for diffing. The default degrades to a full refresh so a
/// missing override stays correct, only slower.
pub trait LeafDiff: Leaf {
    fn equals(&self, other: &dyn Leaf) -> bool {
        let _ = other;
        false
    }
    fn updates(&self, old: Option<&dyn Leaf>) -> Vec<LeafUpdate<'_>> {
        let _ = old;
        vec![LeafUpdate::FullRefresh]
    }
}

/// What `Control` stores: cloneable and diffable leaves.
pub trait ControlLeaf: LeafClone + LeafDiff {}
impl<T: LeafClone + LeafDiff> ControlLeaf for T {}

/// Framework primitive context for `walk`: id plus hints only.
/// No domain or native types travel here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafCtx {
    pub id: ControlId,
    pub hints: LayoutHints,
}

/// Framework-primitive leaf diff. Leaf-specific values collapse into
/// `ValueChanged` (re-walk the leaf); no leaf payload is named here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeafUpdate<'a> {
    SetLabel(&'a str),
    SetEnabled(bool),
    SetVisible(bool),
    SetAccessibleLabel(&'a str),
    SetHelp(Option<&'a str>),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    /// Stable framework id minted from the domain-side key.
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

impl LeafClone for ListLeaf {
    fn clone_box(&self) -> Box<dyn LeafClone> {
        Box::new(self.clone())
    }
}

impl LeafDiff for ListLeaf {
    fn equals(&self, other: &dyn Leaf) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn updates(&self, old: Option<&dyn Leaf>) -> Vec<LeafUpdate<'_>> {
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
    use crate::control::{ControlDisplay, ControlKind, ControlMetadata};

    fn cell(label: &str) -> Control {
        Control {
            id: ControlId::fresh(),
            label: label.into(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            kind: ControlKind::Note { text: label.into() },
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
        impl LeafClone for Other {
            fn clone_box(&self) -> Box<dyn LeafClone> {
                Box::new(Other)
            }
        }
        impl LeafDiff for Other {
            fn equals(&self, _other: &dyn Leaf) -> bool {
                false
            }
        }
        let leaf = ListLeaf::default();
        let other = Other;
        assert!(!LeafDiff::equals(&leaf, &other as &dyn Leaf));
        assert!(LeafDiff::equals(&leaf, &leaf as &dyn Leaf));
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
            LeafDiff::updates(&leaf, None),
            vec![LeafUpdate::FullRefresh]
        );
        assert!(LeafDiff::updates(&leaf, Some(&leaf as &dyn Leaf)).is_empty());
        let mut changed = ListLeaf::default();
        changed.rows.push(ItemRow {
            id: ControlId::fresh(),
            depth: 0,
            expanded: None,
            cells: vec![cell("x")],
        });
        assert_eq!(
            LeafDiff::updates(&changed, Some(&leaf as &dyn Leaf)),
            vec![LeafUpdate::ValueChanged]
        );
    }
}
