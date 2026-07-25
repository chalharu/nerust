use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::{
    EditorState,
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
};

/// A node responsible for computing one projection from [`EditorState`].
pub(crate) trait ProjectionNode {
    fn name(&self) -> &'static str;
    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>>;
    fn is_synced(&self, current: &EditorState) -> bool;
}

/// A prepared value ready to be silently applied to its property cache.
pub(crate) trait PreparedProjection {
    fn apply(self: Box<Self>);
}

/// A concrete PreparedProjection that updates an ObservablePropertyInner.
struct InnerProjection<T: Clone + PartialEq + 'static> {
    inner: Rc<ObservablePropertyInner<T>>,
    value: T,
}

impl<T: Clone + PartialEq + 'static> PreparedProjection for InnerProjection<T> {
    fn apply(self: Box<Self>) {
        self.inner.replace(self.value);
    }
}

/// A concrete ProjectionNode that computes T from EditorState.
struct FuncProjectionNode<T: Clone + PartialEq + 'static> {
    name: &'static str,
    inner: Rc<ObservablePropertyInner<T>>,
    project: Box<dyn Fn(&EditorState) -> T>,
}

impl<T: Clone + PartialEq + 'static> ProjectionNode for FuncProjectionNode<T> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn prepare(&self, candidate: &EditorState) -> Option<Box<dyn PreparedProjection>> {
        let new_value = (self.project)(candidate);
        let old_value = self.inner.get();
        if new_value == old_value {
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
