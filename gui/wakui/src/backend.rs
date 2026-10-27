//! Backend registration and selection (builder style).
//!
//! The composition root builds a [`BackendRegistry`]; each platform
//! frontend registers its static name. Selection prefers an explicit
//! name and falls back to registration order. Window creation and event
//! loops stay native; this registry only names the participants.

/// A platform backend known to the composition root.
pub trait Backend {
    /// Stable name used for selection (e.g. `"gtk"`, `"tao"`).
    fn name(&self) -> &'static str;
}

/// Ordered registry of available backends.
#[derive(Debug, Default)]
pub struct BackendRegistry {
    names: Vec<&'static str>,
}

impl BackendRegistry {
    /// Register a backend name. Duplicates are ignored.
    pub fn register(&mut self, name: &'static str) {
        if !self.names.contains(&name) {
            self.names.push(name);
        }
    }

    /// Names in registration order.
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    /// Preferred name when registered, otherwise the first registered.
    pub fn select(&self, preferred: Option<&str>) -> Option<&'static str> {
        if let Some(wanted) = preferred
            && let Some(found) = self.names.iter().copied().find(|name| *name == wanted)
        {
            return Some(found);
        }
        self.names.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);
    impl Backend for Fake {
        fn name(&self) -> &'static str {
            self.0
        }
    }

    #[test]
    fn registration_dedupes_and_selection_prefers_explicit() {
        let gtk = Fake("gtk");
        let tao = Fake("tao");
        let mut registry = BackendRegistry::default();
        registry.register(gtk.name());
        registry.register(tao.name());
        registry.register(gtk.name());
        assert_eq!(registry.names(), &["gtk", "tao"]);
        assert_eq!(registry.select(Some("tao")), Some("tao"));
        assert_eq!(registry.select(Some("qt")), Some("gtk"));
        assert_eq!(registry.select(None), Some("gtk"));
        let empty = BackendRegistry::default();
        assert_eq!(empty.select(None), None);
    }
}
