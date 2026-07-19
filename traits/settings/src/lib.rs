use std::any::Any;
use std::fmt::Debug;

#[typetag::serde(tag = "system")]
pub trait SystemSettings: Debug + Send + Sync + Any {
    fn requires_live_session_rebuild(&self, next: &dyn SystemSettings) -> bool;
    fn clone_box(&self) -> Box<dyn SystemSettings>;
    fn eq_box(&self, other: &dyn SystemSettings) -> bool;
}

impl Clone for Box<dyn SystemSettings> {
    fn clone(&self) -> Self {
        (**self).clone_box()
    }
}

impl PartialEq for Box<dyn SystemSettings> {
    fn eq(&self, other: &Self) -> bool {
        (**self).eq_box(&**other)
    }
}
