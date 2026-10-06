use nerust_core_traits::factory::CoreFactory;

pub mod error;
pub mod events;
mod factory_adapter;
pub mod harness;
pub mod manifest;
mod media;
pub mod perf;
pub mod report;
pub mod results;
pub mod runner;
mod serde_helpers;
#[cfg(test)]
mod tests;

/// System factories available to headless drivers: one boxed
/// `dyn CoreFactory` per enabled system feature. Empty without
/// features — drivers then ignore every case instead of failing.
/// Construction selects the system once here; everything downstream
/// drives through `dyn CoreFactory`.
pub fn system_factories() -> Vec<Box<dyn CoreFactory>> {
    #[cfg(feature = "nes")]
    return vec![nerust_nes_factory::NesFactory::boxed()];
    #[cfg(not(feature = "nes"))]
    return Vec::new();
}
