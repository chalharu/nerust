//! `wakui`: cross-platform native UI framework core (MAUI/wxWidgets style).
//!
//! The common layer owns control abstractions, handler interfaces, the
//! property mapper and the turn driver. It renders nothing itself:
//! platform backends generate native controls. No intra-repository
//! dependencies; only `std` plus toolkit-independent externals.

pub mod backend;
pub mod binding;
pub mod control;
pub mod key;
pub mod loop_driver;
pub mod mapper;
pub mod viewport;

pub use key::Key;
