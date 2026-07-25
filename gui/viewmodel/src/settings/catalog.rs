use std::{collections::HashMap, sync::Arc};

use nerust_core_traits::{factory::CoreFactory, identity::SystemId};

/// A simple registry of core factories, providing iteration and
/// SystemId-based lookup using trait-object equality.
#[derive(Clone)]
pub(crate) struct FactoryCatalog {
    all: Arc<[Arc<dyn CoreFactory>]>,
    by_id: Arc<HashMap<Box<dyn SystemId>, Arc<dyn CoreFactory>>>,
}

impl FactoryCatalog {
    pub fn new(factories: Vec<Arc<dyn CoreFactory>>) -> Self {
        let mut by_id: HashMap<Box<dyn SystemId>, Arc<dyn CoreFactory>> = HashMap::new();
        for f in &factories {
            let sid = f.system_id();
            if let Some(existing) = by_id.get(&sid) {
                panic!(
                    "duplicate SystemId '{}': {} and {}",
                    sid,
                    existing.display_name(),
                    f.display_name()
                );
            }
            by_id.insert(sid, Arc::clone(f));
        }
        Self {
            all: factories.into(),
            by_id: Arc::new(by_id),
        }
    }

    pub fn all(&self) -> &[Arc<dyn CoreFactory>] {
        &self.all
    }

    pub fn find_by_id(&self, id: &dyn SystemId) -> Option<&Arc<dyn CoreFactory>> {
        self.by_id.get(id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nerust_core_traits::factory::CoreFactory;

    use super::FactoryCatalog;
    use crate::settings::test_support::{TestCoreFactory, TestInputFactory, TestSystemId};

    #[test]
    fn empty_catalog() {
        let c = FactoryCatalog::new(vec![]);
        assert!(c.all().is_empty());
        assert!(c.find_by_id(&TestSystemId).is_none());
    }

    #[test]
    fn single_factory_findable_by_id() {
        let f: Arc<dyn CoreFactory> = Arc::new(TestCoreFactory(TestInputFactory::new()));
        let c = FactoryCatalog::new(vec![Arc::clone(&f)]);
        assert_eq!(c.all().len(), 1);
        assert!(c.find_by_id(&TestSystemId).is_some());
    }

    #[test]
    #[should_panic(expected = "duplicate SystemId")]
    fn duplicate_id_panics() {
        let f: Arc<dyn CoreFactory> = Arc::new(TestCoreFactory(TestInputFactory::new()));
        let f2: Arc<dyn CoreFactory> = Arc::new(TestCoreFactory(TestInputFactory::new()));
        let _c = FactoryCatalog::new(vec![f, f2]);
    }
}
