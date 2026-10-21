//! Shared frontend kernel: hashes, caches, and gates behind both native frontends.
//! Toolkit-free so tao, gtk, and headless all depend downward on it.

pub mod cache;
pub mod hash;
pub mod history;

pub use cache::Versioned;
pub use hash::Fnv1a;
pub use history::level_visible;
