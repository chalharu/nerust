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
