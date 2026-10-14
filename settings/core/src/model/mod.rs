//! Settings domain model: language, bindings, shared/local/app state.
//! Pure data; interpretation lives in the sibling modules. GUI-agnostic
//! so the dependency points inward.

pub mod app_state;
pub mod input;
pub mod language;
pub mod local;
pub mod shared;
pub mod snapshot;

#[cfg(test)]
mod tests;
