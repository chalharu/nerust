use std::any::Any;
use std::fmt::Debug;

use dyn_clone::DynClone;
use dyn_eq::DynEq;

#[typetag::serde(tag = "system")]
pub trait SystemSettings: Debug + Send + Sync + Any + DynClone + DynEq {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool;
}

dyn_clone::clone_trait_object!(SystemSettings);
dyn_eq::eq_trait_object!(SystemSettings);
