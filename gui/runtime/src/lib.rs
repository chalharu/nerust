pub mod rom;
pub mod rom_library;
pub mod settings;
pub mod shell;
pub mod slots;

#[cfg(test)]
mod test {
    use nerust_core_traits::{declare_system_id, identity::SystemId};

    declare_system_id!(DummySystemId, "dummy");
    declare_system_id!(DummyOtherSystemId, "other");
}
