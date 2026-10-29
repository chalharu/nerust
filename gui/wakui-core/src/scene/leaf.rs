//! Extensible leaf controls (trait objects).
//!
//! `Control` stores a boxed [`ControlLeaf`]; base leaves and the generic
//! [`ListLeaf`] are framework-provided, domain leaves are implemented
//! outside `wakui`. Flavors render through [`ControlVisitor`](crate::scene::control::ControlVisitor)
//! plus `as_any` downcasting and never match on view-definition types.
//!
//! Dependency direction: leaves and containers live in `wakui` and know
//! only framework types. `debug_name` is diagnostics only, never a dispatch key.

mod base;
mod core;
mod list;

pub use base::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, ImageLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};
pub use core::{ControlLeaf, Leaf, LeafCtx, LeafUpdate};
pub use list::{ColumnDef, ItemRow, ListLeaf};
