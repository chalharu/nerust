//! `wakui`: cross-platform native UI framework core (MAUI/wxWidgets style).
//!
//! The common layer owns control abstractions, handler interfaces, the
//! property mapper and the turn driver. It renders nothing itself:
//! platform backends generate native controls. No intra-repository
//! dependencies; only `std` plus toolkit-independent externals.
//!
//! # Writing a backend
//!
//! 1. Depend on `wakui` only.
//! 2. Render [`control::ControlScreen`] snapshots by implementing
//!    [`control::ControlVisitor`]. Never match on view-definition types.
//! 3. Apply [`mapper::map_updates`] lists (`None` for construction,
//!    `Some(previous)` for retained patching); register a
//!    [`mapper::MapperRule`] for native quirks instead of forking updates.
//! 4. Drive end-of-burst turns through [`loop_driver::drive_turn`] with
//!    your [`loop_driver::LoopHost`] implementation.
//! 5. Register your static [`backend::Backend::name`] in the composition
//!    root's [`backend::BackendRegistry`] for builder-style selection.
//! 6. Deliver native picker results through [`binding::PickerSink`].

pub mod backend;
pub mod binding;
pub mod cache;
pub mod control;
pub mod key;
pub mod loop_driver;
pub mod mapper;
pub mod pause;
pub mod retry;
pub mod viewport;

pub use key::Key;
