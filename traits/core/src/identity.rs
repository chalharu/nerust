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
        let value: &dyn ToString = self;
        f.write_str(&value.to_string())
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
    pub use typetag;
}

#[macro_export]
macro_rules! declare_system_id {
    // The persisted typetag is the globally unique logical system ID. Keep it
    // stable and declare each system ID exactly once in the final binary.
    ($visibility:vis $name:ident, $system_id:literal) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            $crate::identity::__private::_serde_serialize,
            $crate::identity::__private::_serde_deserialize,
        )]
        $visibility struct $name;

        const _: () = {
            // typetag's proc macro emits `typetag::...` paths at the call site.
            // Keep that implementation detail available without requiring the
            // calling crate to depend on typetag directly.
            use $crate::identity::__private::typetag;

            #[$crate::identity::__private::typetag::serde(name = $system_id)]
            impl $crate::identity::SystemId for $name {}
        };

        #[allow(clippy::to_string_trait_impl)]
        impl ToString for $name {
            fn to_string(&self) -> String {
                $system_id.to_string()
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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::SystemId;

    crate::declare_system_id!(FirstSystemId, "first");
    crate::declare_system_id!(RenamableRustType, "second");

    fn serialized_tag(system_id: Box<dyn SystemId>) -> String {
        let Value::Object(object) = serde_json::to_value(system_id).unwrap() else {
            panic!("system ID should serialize as an object");
        };
        object.get("sid").unwrap().as_str().unwrap().to_string()
    }

    #[test]
    fn system_id_typetag_uses_stable_logical_id() {
        assert_eq!(serialized_tag(Box::new(FirstSystemId)), "first");
        assert_eq!(serialized_tag(Box::new(RenamableRustType)), "second");
    }

    #[test]
    fn system_id_typetag_round_trips() {
        let encoded =
            serde_json::to_string(&(Box::new(FirstSystemId) as Box<dyn SystemId>)).unwrap();
        let decoded: Box<dyn SystemId> = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.as_ref(), &FirstSystemId as &dyn SystemId);
    }
}
