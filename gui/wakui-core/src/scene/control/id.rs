//! Opaque process-unique identity for controls, layouts, and pages.
//! Issued tokens are never recycled; controllers keep stable ids for
//! surviving keys through [`IdRegistry`].

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-unique opaque token. Neither position nor a hash of a domain key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(u64);
impl Id {
    /// Mint a fresh process-unique token. Always safe: issued IDs are never
    /// recycled. Prefer [`IdRegistry`] so surviving keys keep stable IDs.
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut value = NEXT.load(Ordering::Relaxed);
        loop {
            let next = value.checked_add(1).expect("control identity exhausted");
            match NEXT.compare_exchange_weak(value, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Self(value),
                Err(current) => value = current,
            }
        }
    }
}
pub type ControlId = Id;
pub type LayoutId = Id;
pub type PageId = Id;
pub type TabsId = Id;
pub type TabId = Id;

/// Controller-owned equality registry. Issued IDs are never reassigned or recycled.
/// Keys keep their IDs while retained; a removed key gets a fresh ID if it returns.
/// The caller supplies semantic keys, not indices in a changing collection.
#[derive(Debug)]
pub struct IdRegistry<K>(Vec<(K, Id)>);

impl<K> Default for IdRegistry<K> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: Eq> IdRegistry<K> {
    /// Drop obsolete keys without changing surviving IDs or recycling issued IDs.
    /// Do not remove keys whose identity must survive temporarily hidden topology.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, Id) -> bool) {
        self.0.retain(|(key, id)| keep(key, *id));
    }

    pub fn id(&mut self, key: K) -> Id {
        if let Some((_, id)) = self.0.iter().find(|(existing, _)| existing == &key) {
            return *id;
        }
        let id = Id::fresh();
        self.0.push((key, id));
        id
    }
}
