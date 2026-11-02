pub mod debug_transaction;
pub mod debugger;
pub mod menu_labels;
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

    /// Test-only system settings: mirrors the NES filter contract the
    /// fixtures previously exercised through the real `NesSettings`
    /// (`visual` flips require a live rebuild, `non_visual` flips do not),
    /// so even tests stay free of concrete system crates.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
    pub(crate) struct DummySystemSettings {
        pub(crate) visual: u8,
        pub(crate) non_visual: u8,
    }

    #[typetag::serde(name = "dummy_system_settings")]
    impl nerust_settings_traits::SystemSettings for DummySystemSettings {
        fn requires_live_session_rebuild(
            &self,
            next: &dyn nerust_settings_traits::SystemSettings,
        ) -> bool {
            next.downcast_ref::<DummySystemSettings>()
                .is_some_and(|other| self.visual != other.visual)
        }
    }
}
