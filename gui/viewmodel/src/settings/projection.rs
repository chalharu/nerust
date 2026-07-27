use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::{
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
    state::EditorState,
};

/// A node responsible for computing one projection from [`EditorState`].
pub trait ProjectionNode {
    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>>;
}

type ApplyNotification = Option<Box<dyn FnOnce()>>;

/// A prepared value ready to be silently applied to its property cache.
/// Returns an optional notification closure to be invoked after all
/// projections have been applied.
pub trait PreparedProjection {
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
    inner: Rc<ObservablePropertyInner<T>>,
    project: Box<dyn Fn(&EditorState) -> T>,
}

impl<T: Clone + PartialEq + 'static> ProjectionNode for FuncProjectionNode<T> {
    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>> {
        let new_value = (self.project)(candidate);
        if self.inner.get() == new_value {
            return None;
        }
        Some(Box::new(InnerProjection {
            inner: Rc::clone(&self.inner),
            value: new_value,
        }))
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
            inner: Rc::clone(&inner),
            project: Box::new(project),
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
}
