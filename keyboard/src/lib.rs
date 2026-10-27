//! Re-export shim for the framework-owned key type.
//! Converters moved into the frontends in Phase 2 (`wakui` owns `Key`;
//! orphan rules forbid trait impls for it elsewhere).

pub use wakui::Key;
