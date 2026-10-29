//! Open arrangement vocabulary: layout is data-driven.
//! New arrangements implement [`LayoutPolicy`]; containers never grow
//! new enum variants for them.

use std::any::Any;

/// Open arrangement vocabulary. New arrangements implement this trait;
/// containers never grow new enum variants for them.
pub trait LayoutPolicy: std::fmt::Debug + Send + 'static {
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    fn clone_box(&self) -> Box<dyn LayoutPolicy>;
    fn equals(&self, other: &dyn LayoutPolicy) -> bool;
    fn as_any(&self) -> &dyn Any;
}

macro_rules! unit_policy {
    ($t:ty) => {
        impl LayoutPolicy for $t {
            fn clone_box(&self) -> Box<dyn LayoutPolicy> {
                Box::new(*self)
            }
            fn equals(&self, other: &dyn LayoutPolicy) -> bool {
                other.as_any().downcast_ref::<Self>().is_some()
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

/// Horizontal stack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowPolicy;
/// Vertical stack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnPolicy;
/// Framed group with an optional header control.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupPolicy;
/// Grid arrangement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TablePolicy;
/// Flowing arrangement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlexPolicy;
/// Absolute arrangement. The only policy carrying a rectangle, and only
/// for exceptional overlay use; ordinary screens never set it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AbsolutePolicy {
    pub bounds: Option<crate::scene::hints::Rect>,
}

unit_policy!(RowPolicy);
unit_policy!(ColumnPolicy);
unit_policy!(GroupPolicy);
unit_policy!(TablePolicy);
unit_policy!(FlexPolicy);

impl LayoutPolicy for AbsolutePolicy {
    fn clone_box(&self) -> Box<dyn LayoutPolicy> {
        Box::new(*self)
    }
    fn equals(&self, other: &dyn LayoutPolicy) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
