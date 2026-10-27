//! Generic view/binder contracts shared by every domain.
//! Phase 1: host-internal. Phase 2: move to the `wakui` framework crate.

/// Pure view side: input validation and domain-event mapping.
/// Native code and controller ownership never appear here.
pub trait ViewDefinition {
    /// Canonical registry identity, not a localized label or positional key.
    /// Linear lookup suffices; no hash implementation is required.
    type ViewId: Clone + Eq;
    type ViewInput;
    type DomainEvent;
    /// Resolve a retained ID against current values before handling input.
    fn bind(&self, id: &Self::ViewId, input: Self::ViewInput) -> Option<Vec<Self::DomainEvent>>;
}

/// Bound instance side: apply domain events and return native effects.
pub trait BindingTarget {
    type DomainEvent;
    type Effect;
    fn apply(&mut self, event: Self::DomainEvent) -> Option<Self::Effect>;
}

/// Reverse notification: the instance tells views their snapshot is stale.
/// Native registries poll this instead of borrowing controller state.
pub trait Invalidatable {
    fn invalidate(&self);
    fn take_invalidated(&self) -> bool;
}
