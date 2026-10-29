//! Leaf core: the [`Leaf`] render trait, the stored [`ControlLeaf`],
//! the [`LeafCtx`] walk context, and the [`LeafUpdate`] diff.

use std::any::Any;

use crate::scene::control::{ControlId, ControlInput, ControlVisitor};
use crate::scene::hints::LayoutHints;

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
