//! Backend registration for the gtk4 platform handler.

#[cfg(test)]
use wakui_core::app::backend::Backend;
use wakui_core::app::backend::BackendRegistry;

/// Stable backend name used for composition-root selection.
pub(crate) const NAME: &str = "gtk";

/// Marker backend proving the gtk4 handler contract (test-only proof).
#[cfg(test)]
pub(crate) struct GtkBackend;

#[cfg(test)]
impl Backend for GtkBackend {
    fn name(&self) -> &'static str {
        NAME
    }
}

/// Register the gtk backend; duplicates are ignored.
pub fn register(registry: &mut BackendRegistry) {
    registry.register(NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_backend_registers_under_its_stable_name() {
        let mut registry = BackendRegistry::default();
        register(&mut registry);
        register(&mut registry);
        assert_eq!(registry.names(), &["gtk"]);
        assert_eq!(registry.select(Some("gtk")), Some("gtk"));
        assert_eq!(GtkBackend.name(), "gtk");
    }
}
