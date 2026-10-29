//! Composition boundary: the framework app trait, backend selection, and
//! view/binder contracts. The composition root injects the view-side app
//! through these types; flavors never name view definitions.
pub mod backend;
pub mod binding;
pub mod boundary;
pub mod launch;
pub mod roles;
