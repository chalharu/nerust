pub mod rom;
pub mod rom_library;
pub mod settings;
pub mod shell;
pub mod slots;

#[cfg(test)]
mod test {
    use std::{any::TypeId, hash::Hash};

    use nerust_core_traits::identity::SystemId;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
    pub(crate) struct DummySystemId;

    #[typetag::serde]
    impl SystemId for DummySystemId {}

    impl ToString for DummySystemId {
        fn to_string(&self) -> String {
            "dummy".to_string()
        }
    }

    impl Hash for DummySystemId {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            TypeId::of::<Self>().hash(state);
        }
    }

    #[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
    pub(crate) struct DummyOtherSystemId;

    #[typetag::serde]
    impl SystemId for DummyOtherSystemId {}

    impl ToString for DummyOtherSystemId {
        fn to_string(&self) -> String {
            "dummy".to_string()
        }
    }

    impl Hash for DummyOtherSystemId {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            TypeId::of::<Self>().hash(state);
        }
    }
}
