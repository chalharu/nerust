//! Settings domain model: language, input bindings, shared/local/app
//! state, and the composite snapshot. Pure data with serde shapes;
//! interpretation lives in the sibling modules (`bindings`, `editor`,
//! `factory`, `i18n`).
//!
//! Moved down from `nerust_gui_settings` so the dependency points
//! inward (domain logic and GUI alike consume the model; nothing in
//! the model names a GUI toolkit).

pub mod app_state;
pub mod input;
pub mod language;
pub mod local;
pub mod shared;
pub mod snapshot;

#[cfg(test)]
mod tests;
