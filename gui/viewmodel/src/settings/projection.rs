#![allow(dead_code)]

use std::{
    cell::RefCell,
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
    fn apply(self: Box<Self>) -> Option<Vec<Rc<dyn Fn(&Box<dyn std::any::Any>)>>>;
}

/// Registry of all projection nodes, evaluated on every transaction.
#[derive(Clone)]
pub(crate) struct ProjectionHub {
    nodes: Rc<RefCell<Vec<Rc<dyn ProjectionNode>>>>,
}

impl ProjectionHub {
    pub fn new() -> Self {
        Self {
            nodes: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn register<T, P>(&self, initial: T, _project: P) -> ReadOnlyObservableProperty<T>
    where
        T: Clone + PartialEq + 'static,
        P: Fn(&EditorState) -> T + 'static,
    {
        let inner = Rc::new(ObservablePropertyInner::new(initial));
        ReadOnlyObservableProperty::new(inner)
    }

    pub fn prepare_all(&self, candidate: &EditorState) -> Vec<Box<dyn PreparedProjection>> {
        let nodes = self.nodes.borrow();
        nodes
            .iter()
            .filter_map(|node| node.prepare(candidate))
            .collect()
    }

    pub fn seal(&self) {}

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
