//! `wakui_core`: cross-platform native UI framework core (MAUI/wxWidgets style).
//!
//! The common layer owns control abstractions, handler interfaces, the
//! property mapper and the turn driver. It renders nothing itself:
//! platform backends generate native controls. No intra-repository
//! dependencies; only `std` plus toolkit-independent externals.
//!
//! # Writing a backend
//!
//! 1. Depend on `wakui_core` only.
//! 2. Render [`scene::control::ControlScreen`] snapshots by implementing
//!    [`scene::control::ControlVisitor`]. Never match on view-definition types.
//! 3. Apply [`flow::mapper::map_updates`] lists (`None` for construction,
//!    `Some(previous)` for retained patching); register a
//!    [`flow::mapper::MapperRule`] for native quirks instead of forking updates.
//! 4. Drive end-of-burst turns through [`flow::loop_driver::drive_turn`] with
//!    your [`flow::loop_driver::LoopHost`] implementation.
//! 5. Register your static [`app::backend::Backend::name`] in the composition
//!    root's [`app::backend::BackendRegistry`] for builder-style selection.
//! 6. Deliver native picker results through [`app::binding::PickerSink`].

pub mod app;
pub mod chrome;
pub mod flow;
pub mod input;
pub mod scene;
pub mod support;

pub use input::key::Key;
