//! Framework-provided generic list: covers list/table/tree views with one
//! type, so no per-domain row enum grows.

use std::any::Any;

use crate::scene::control::{Control, ControlId, ControlVisitor};

use super::core::{ControlLeaf, Leaf, LeafCtx, LeafUpdate};

/// Generic column definition for `ListLeaf`. Headers arrive
/// pre-localized; widths are optional hints (`None` = auto).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
        // Item rows group their cells: flavors that can nest build one
        // horizontal row per item (disassembly line plus follow arrow,
        // dump address plus byte buttons). Visitors without row support
        // keep the legacy flat cell order through the default no-ops.
        let _ = ctx;
        for row in &self.rows {
            visitor.begin_row(row);
            for cell in &row.cells {
                visitor.control(cell);
            }
            visitor.end_row(row);
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
    use crate::scene::control::{ControlDisplay, ControlMetadata};
    use crate::scene::hints::LayoutHints;
    use crate::scene::leaf::NoteLeaf;

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
