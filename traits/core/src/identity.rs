use std::fmt::{Debug, Display};

use downcast_rs::Downcast;
use dyn_clone::DynClone;
use dyn_eq::DynEq;
use dyn_hash::DynHash;

/// システム識別子。CoreFactory impl のみが生成する。
/// 比較は `Eq` 経由のみ。生文字列の取り出しは不可。
#[typetag::serde(tag = "sid")]
pub trait SystemId: Debug + DynClone + DynEq + DynHash + Downcast + Send + Sync + ToString {}

dyn_clone::clone_trait_object!(SystemId);
dyn_eq::eq_trait_object!(SystemId);
dyn_hash::hash_trait_object!(SystemId);
downcast_rs::impl_downcast!(SystemId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemIdentity {
    pub system_id: Box<dyn SystemId>,
    pub identity_bytes: Vec<u8>,
}

impl SystemIdentity {
    pub fn new(system_id: Box<dyn SystemId>, identity_bytes: Vec<u8>) -> Self {
        Self {
            system_id,
            identity_bytes,
        }
    }
}

impl Display for dyn SystemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

impl dyn SystemId {
    pub fn clone_box(&self) -> Box<dyn SystemId> {
        dyn_clone::clone_box(self)
    }
}
