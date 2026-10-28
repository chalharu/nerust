//! Generic view/binder contracts shared by every domain.
//! Implemented by view definitions (`SettingsScreen`, `HistoryModel`),
//! bound instances (`SettingsController`, `HistorySession`) and native
//! picker sinks. See the `wakui` separation design for the full picture.

use crate::control::{ControlId, ControlInput, Id};

/// Pure view side: input validation and domain-event mapping.
/// Native code and controller ownership never appear here.
pub trait ViewDefinition {
    /// Canonical registry identity, not a localized label or positional key.
    /// Linear lookup suffices; no hash implementation is required.
    type ViewId: Clone + Eq;
    type ViewInput;
    type DomainEvent;
    /// Convert a retained-control input into domain input before binding.
    /// Partial: return `None` for inputs the view does not handle, so a
    /// generic dispatcher can reject before resolving any domain state.
    fn convert(input: ControlInput) -> Option<Self::ViewInput>;
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

/// File-picker result delivery. The framework owns request correlation,
/// but never names filesystem paths: the payload type stays generic so
/// platform adapters depend only on `wakui`, not on view definitions.
pub trait PickerSink<Payload> {
    fn submit(&mut self, target: ControlId, request: Id, payload: Option<Payload>);
}
