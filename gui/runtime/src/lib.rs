pub mod rom;
pub mod rom_library;
pub mod settings;
pub mod shell;
pub mod slots;

#[cfg(test)]
mod test {
    use nerust_core_traits::declare_system_id;

    declare_system_id!(pub(crate) DummySystemId, "dummy");
    declare_system_id!(pub(crate) DummyOtherSystemId, "other");
}
