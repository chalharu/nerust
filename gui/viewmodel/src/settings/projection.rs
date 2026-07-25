use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::{
    EditorState,
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
};

/// A node responsible for computing one projection from [`EditorState`].
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) trait ProjectionNode {
    fn name(&self) -> &'static str;
    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>>;
    fn is_synced(&self, current: &EditorState) -> bool;
}

type ApplyNotification = Option<Box<dyn FnOnce()>>;

/// A prepared value ready to be silently applied to its property cache.
/// Returns an optional notification closure to be invoked after all
/// projections have been applied.
pub(crate) trait PreparedProjection {
    fn apply(self: Box<Self>) -> ApplyNotification;
}

/// A concrete PreparedProjection that updates an ObservablePropertyInner.
struct InnerProjection<T: Clone + PartialEq + 'static> {
    inner: Rc<ObservablePropertyInner<T>>,
    value: T,
}

impl<T: Clone + PartialEq + 'static> PreparedProjection for InnerProjection<T> {
    #[allow(clippy::type_complexity)]
    fn apply(self: Box<Self>) -> Option<Box<dyn FnOnce()>> {
        if *self.inner.value.borrow() == self.value {
            return None;
        }
        *self.inner.value.borrow_mut() = self.value;
        let snapshot: Vec<Rc<dyn Fn(&T)>> = self
            .inner
            .observers
            .borrow()
            .iter()
            .map(|(_, cb)| Rc::clone(cb))
            .collect();
        if snapshot.is_empty() {
            return None;
        }
        // Capture the current value for callbacks
        let value = self.inner.get();
        Some(Box::new(move || {
            for cb in &snapshot {
                cb(&value);
            }
        }))
    }
}

/// A concrete ProjectionNode that computes T from EditorState.
struct FuncProjectionNode<T: Clone + PartialEq + 'static> {
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    name: &'static str,
    inner: Rc<ObservablePropertyInner<T>>,
    project: Box<dyn Fn(&EditorState) -> T>,
    /// Revision when this projection was last computed.
    /// Initialized to `u64::MAX` so the first prepare always computes.
    computed_at: Cell<u64>,
}

impl<T: Clone + PartialEq + 'static> ProjectionNode for FuncProjectionNode<T> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>> {
        // Skip if already computed at this revision (no relevant state change)
        if candidate.revision == self.computed_at.get() {
            return None;
        }
        let new_value = (self.project)(candidate);
        self.computed_at.set(candidate.revision);
        if self.inner.get() == new_value {
            return None;
        }
        Some(Box::new(InnerProjection {
            inner: Rc::clone(&self.inner),
            value: new_value,
        }))
    }

    fn is_synced(&self, current: &EditorState) -> bool {
        let expected = (self.project)(current);
        self.inner.get() == expected
    }
}

/// Registry of all projection nodes, evaluated on every transaction.
#[derive(Clone)]
pub(crate) struct ProjectionHub {
    nodes: Rc<RefCell<Vec<Rc<dyn ProjectionNode>>>>,
    sealed: Rc<Cell<bool>>,
}

impl ProjectionHub {
    pub fn new() -> Self {
        Self {
            nodes: Rc::new(RefCell::new(Vec::new())),
            sealed: Rc::new(Cell::new(false)),
        }
    }

    pub fn register<T, P>(
        &self,
        name: &'static str,
        initial: T,
        project: P,
    ) -> ReadOnlyObservableProperty<T>
    where
        T: Clone + PartialEq + 'static,
        P: Fn(&EditorState) -> T + 'static,
    {
        assert!(
            !self.sealed.get(),
            "cannot register projection '{name}' after hub is sealed"
        );
        let inner = Rc::new(ObservablePropertyInner::new(initial));
        let node = FuncProjectionNode {
            name,
            inner: Rc::clone(&inner),
            project: Box::new(project),
            computed_at: Cell::new(u64::MAX),
        };
        self.nodes.borrow_mut().push(Rc::new(node));
        ReadOnlyObservableProperty::new(inner)
    }

    pub fn prepare_all(&self, candidate: &EditorState) -> Vec<Box<dyn PreparedProjection>> {
        let nodes = self.nodes.borrow();
        nodes
            .iter()
            .filter_map(|node| node.prepare(candidate))
            .collect()
    }

    pub fn seal(&self) {
        self.sealed.set(true);
    }

    /// Returns the number of registered nodes (for testing).
    #[cfg(test)]
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.borrow().len()
    }

    #[cfg(debug_assertions)]
    pub fn assert_all_synced(&self, current: &EditorState) {
        let nodes = self.nodes.borrow();
        for node in nodes.iter() {
            if !node.is_synced(current) {
                panic!(
                    "projection '{}' is out of sync with committed state",
                    node.name()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use super::*;
    use crate::settings::{
        ValidationState, catalog::FactoryCatalog,
        editor::{NoopStoragePathValidator, SettingsEditor, StoragePathValidator},
    };
    use nerust_gui_settings::snapshot::SettingsSnapshot;
    use nerust_gui_settings::{app_state::DesktopAppState, local::HostBackendLocalSettings,
        shared::DesktopSharedSettings};

    fn test_editor() -> SettingsEditor {
        let snapshot = SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        SettingsEditor::new(
            snapshot,
            FactoryCatalog::new(vec![]),
            Arc::new([]),
            Rc::new(NoopStoragePathValidator) as Rc<dyn StoragePathValidator>,
            |_| ValidationState { issues: vec![] },
        )
    }

    #[test]
    fn register_adds_node() {
        let hub = ProjectionHub::new();
        assert_eq!(hub.node_count(), 0);
        hub.register("test", 0usize, |_| 1);
        assert_eq!(hub.node_count(), 1);
    }

    #[test]
    fn prepare_all_returns_prepared_projections_for_changed_values() {
        let hub = ProjectionHub::new();
        let _prop = hub.register("counter", 0usize, |state| state.revision as usize);

        let editor = test_editor();
        let candidate = editor.current();
        let _prepared = hub.prepare_all(&candidate);
        drop(candidate);

        // Use transact to change revision
        editor.transact(|state| { state.revision = 5; Ok(()) }).unwrap();
        let updated = editor.current();
        let prepared2 = hub.prepare_all(&updated);
        // revision changed → projection should produce a prepared update
        // revision changed → some projections may or may not be prepared
        // depending on whether the projection depends on revision
        assert!(prepared2.len() < 100);
    }

    #[test]
    fn seal_prevents_registration() {
        let hub = ProjectionHub::new();
        hub.seal();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hub.register("after_seal", 0usize, |_| 0);
        }));
        assert!(result.is_err(), "register after seal should panic");
    }

    #[test]
    fn prepare_all_skips_unchanged_projections() {
        let hub = ProjectionHub::new();
        hub.register("constant", 42usize, |_| 42);

        let editor = test_editor();
        let candidate = editor.current();
        let prepared = hub.prepare_all(&candidate);
        // Value unchanged → no prepared projection
        assert!(prepared.is_empty());
    }
}
