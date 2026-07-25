#![allow(dead_code)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type ObserverList<T> = Vec<(u64, Rc<dyn Fn(&T)>)>;
type CallbackSnapshot<T> = Vec<Rc<dyn Fn(&T)>>;

/// A read-only handle to an observable property.
///
/// Callers can read the current value via `get()` and subscribe to
/// changes via `observe()`. They cannot set the value directly.
pub struct ReadOnlyObservableProperty<T: Clone + PartialEq + 'static> {
    inner: Rc<ObservablePropertyInner<T>>,
}

impl<T: Clone + PartialEq + 'static> Clone for ReadOnlyObservableProperty<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T: Clone + PartialEq + 'static> ReadOnlyObservableProperty<T> {
    pub(crate) fn new(inner: Rc<ObservablePropertyInner<T>>) -> Self {
        Self { inner }
    }

    pub fn get(&self) -> T {
        self.inner.value.borrow().clone()
    }

    pub fn observe(&self, callback: impl Fn(&T) + 'static) -> Subscription {
        let mut observers = self.inner.observers.borrow_mut();
        let id = self.inner.next_observer_id.get();
        self.inner.next_observer_id.set(id + 1);
        observers.push((id, Rc::new(callback)));
        let weak = Rc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.observers.borrow_mut().retain(|(oid, _)| *oid != id);
            }
        })
    }
}

pub(crate) struct ObservablePropertyInner<T> {
    value: RefCell<T>,
    next_observer_id: Cell<u64>,
    observers: RefCell<ObserverList<T>>,
}

impl<T: Clone + PartialEq + 'static> ObservablePropertyInner<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            next_observer_id: Cell::new(0),
            observers: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// Set the value and return callbacks to invoke if changed.
    /// Returns `None` if the value is unchanged.
    pub(crate) fn set(&self, value: T) -> Option<CallbackSnapshot<T>> {
        if *self.value.borrow() == value {
            return None;
        }
        *self.value.borrow_mut() = value;
        let snapshot = self
            .observers
            .borrow()
            .iter()
            .map(|(_, cb)| Rc::clone(cb))
            .collect();
        Some(snapshot)
    }

    /// Replace the value without notifying observers.
    pub(crate) fn replace(&self, value: T) {
        *self.value.borrow_mut() = value;
    }
}

/// A subscription handle that removes the callback when dropped.
pub struct Subscription {
    unsubscribe: Option<Box<dyn FnOnce()>>,
}

impl Subscription {
    fn new(unsubscribe: impl FnOnce() + 'static) -> Self {
        Self {
            unsubscribe: Some(Box::new(unsubscribe)),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(f) = self.unsubscribe.take() {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_returns_initial_value() {
        let inner = Rc::new(ObservablePropertyInner::new(42u32));
        let prop = ReadOnlyObservableProperty::new(inner);
        assert_eq!(prop.get(), 42);
    }

    #[test]
    fn observe_receives_updates() {
        let inner = Rc::new(ObservablePropertyInner::new(0u32));
        let prop = ReadOnlyObservableProperty::new(inner.clone());
        let observed = Rc::new(Cell::new(0u32));
        let observed_cb = Rc::clone(&observed);
        let sub = prop.observe(move |v| {
            observed_cb.set(*v);
        });
        let callbacks = inner.set(42).expect("should notify");
        for cb in &callbacks {
            cb(&42);
        }
        assert_eq!(observed.get(), 42);
        drop(sub);
        // After subscription drop, no callbacks remain
        let callbacks = inner
            .set(100)
            .expect("value changed, should have callbacks vec");
        // The callbacks vec should be empty
        assert!(callbacks.is_empty());
        // And observed should still be 42 (no callback was called)
        assert_eq!(observed.get(), 42);
    }

    #[test]
    fn set_returns_none_for_same_value() {
        let inner = Rc::new(ObservablePropertyInner::new(42u32));
        assert!(inner.set(42).is_none());
        assert!(inner.set(43).is_some());
    }

    #[test]
    fn subscription_drop_removes_callback() {
        let inner = Rc::new(ObservablePropertyInner::new(0u32));
        let prop = ReadOnlyObservableProperty::new(inner.clone());
        let count = Rc::new(Cell::new(0u32));
        let sub = {
            let count = Rc::clone(&count);
            prop.observe(move |_| {
                count.set(count.get() + 1);
            })
        };
        drop(sub);
        let callbacks = inner.set(42);
        assert_eq!(callbacks.map(|cbs| cbs.len()).unwrap_or(0), 0);
    }

    #[test]
    fn observe_does_not_notify_immediately() {
        let inner = Rc::new(ObservablePropertyInner::new(42u32));
        let prop = ReadOnlyObservableProperty::new(inner);
        let observed = Rc::new(Cell::new(None::<u32>));
        let observed_cb = Rc::clone(&observed);
        let _sub = prop.observe(move |v| {
            observed_cb.set(Some(*v));
        });
        // No notification should have happened yet
        assert_eq!(observed.get(), None);
    }
}
