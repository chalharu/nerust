//! Backend registration for the iced/tao platform handler.

#[cfg(test)]
use wakui_core::app::backend::Backend;
use wakui_core::app::backend::BackendRegistry;

/// Stable backend name used for composition-root selection.
pub(crate) const NAME: &str = "tao";

/// Marker backend proving the iced/tao handler contract (test-only proof).
#[cfg(test)]
pub(crate) struct TaoBackend;

#[cfg(test)]
impl Backend for TaoBackend {
    fn name(&self) -> &'static str {
        NAME
    }
}

/// Register the tao backend; duplicates are ignored.
pub fn register(registry: &mut BackendRegistry) {
    registry.register(NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tao_backend_registers_under_its_stable_name() {
        let mut registry = BackendRegistry::default();
        register(&mut registry);
        register(&mut registry);
        assert_eq!(registry.names(), &["tao"]);
        assert_eq!(registry.select(Some("tao")), Some("tao"));
        assert_eq!(TaoBackend.name(), "tao");
    }
}
