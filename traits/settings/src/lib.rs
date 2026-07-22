use std::fmt::Debug;

use downcast_rs::Downcast;
use dyn_clone::DynClone;
use dyn_eq::DynEq;

#[typetag::serde(tag = "system")]
pub trait SystemSettings: Debug + Send + Sync + DynClone + DynEq + Downcast {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool;
}

downcast_rs::impl_downcast!(SystemSettings);
dyn_clone::clone_trait_object!(SystemSettings);
dyn_eq::eq_trait_object!(SystemSettings);
