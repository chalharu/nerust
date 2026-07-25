use std::{collections::HashMap, sync::Arc};

use nerust_core_traits::{factory::CoreFactory, identity::SystemId};

/// A simple registry of core factories, providing iteration and
/// SystemId-based lookup without depending on the full SystemRegistry.
#[derive(Clone)]
pub(crate) struct FactoryCatalog {
    all: Arc<[Arc<dyn CoreFactory>]>,
    by_id: Arc<HashMap<String, Arc<dyn CoreFactory>>>,
}

impl FactoryCatalog {
    pub fn new(factories: Vec<Arc<dyn CoreFactory>>) -> Self {
        let by_id: HashMap<String, Arc<dyn CoreFactory>> = factories
            .iter()
            .map(|f| (f.system_id().to_string(), Arc::clone(f)))
            .collect();
        Self {
            all: factories.into(),
            by_id: Arc::new(by_id),
        }
    }

    pub fn all(&self) -> &[Arc<dyn CoreFactory>] {
        &self.all
    }

    pub fn find_by_id(&self, id: &dyn SystemId) -> Option<&Arc<dyn CoreFactory>> {
        self.by_id.get(&id.to_string())
    }
}
