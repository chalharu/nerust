use std::fmt::{Debug, Display};

use dyn_clone::DynClone;
use dyn_eq::DynEq;
use dyn_hash::DynHash;

/// システム識別子。CoreFactory impl のみが生成する。
/// 比較は `Eq` 経由のみ。生文字列の取り出しは不可。
#[typetag::serde(tag = "sid")]
pub trait SystemId: Debug + DynClone + DynEq + DynHash + Send + Sync + ToString {}

dyn_clone::clone_trait_object!(SystemId);
dyn_eq::eq_trait_object!(SystemId);
dyn_hash::hash_trait_object!(SystemId);

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

pub mod __private {
    pub use serde::Deserialize as _serde_deserialize;
    pub use serde::Serialize as _serde_serialize;
    pub use typetag::serde as _typetag_serde;
}

#[macro_export]
macro_rules! declare_system_id {
    ($name:ident, $value:expr) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            $crate::identity::__private::_serde_serialize,
            $crate::identity::__private::_serde_deserialize,
        )]
        pub(crate) struct $name;

        #[$crate::identity::__private::_typetag_serde]
        impl SystemId for $name {}

        impl ToString for $name {
            fn to_string(&self) -> String {
                $value.to_string()
            }
        }

        impl core::hash::Hash for $name {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                core::any::TypeId::of::<Self>().hash(state);
            }
        }

        impl PartialEq for $name {
            fn eq(&self, _other: &Self) -> bool {
                true
            }
        }
        
        impl Eq for $name {}
    };
}
